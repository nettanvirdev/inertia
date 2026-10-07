//! Containers on this machine, driven through the `docker` CLI.
//!
//! The CLI rather than the daemon socket on purpose. It is the interface Docker
//! documents and keeps stable, it works the same against Docker Desktop, Colima
//! and a remote context without any of them being special-cased here, and it
//! means a person debugging this can paste the same command into a terminal and
//! see what the app saw.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine;
use tokio::io::AsyncWriteExt;

use crate::image;
use crate::{
    machine_name, shell_quote, ComputerError, Created, DirEntry, ExecRequest, ExecResult, Health,
    Provider, Readiness, Result, Screen, Snapshot, Spec, Stats, Status, DEFAULT_TIMEOUT,
};

/// The live screen, published to loopback only.
const SCREEN_VIEW_PORT: u16 = 6080;
const SCREEN_CONTROL_PORT: u16 = 6081;

/// Where the container's `start.sh` reads the screen's password from.
const SCREEN_PASSWORD_ENV: &str = "INERTIA_SCREEN_PASSWORD";

/// The label a snapshot carries naming the machine it was taken of, so one
/// machine's snapshots are neither listed for nor restored onto another.
const MACHINE_LABEL: &str = "inertia.machine";

/// The most processes and threads one machine may hold at once.
///
/// A fork bomb, or a build tool that spawns without bound, otherwise takes the
/// whole Docker VM down with it and every other machine on it. Chromium alone
/// runs a few hundred threads, and Docker counts threads, so this is set well
/// above what a browser plus a parallel build reaches rather than at a number
/// that sounds tidy.
const PIDS_LIMIT: &str = "4096";

/// A fresh password for one machine's screen.
///
/// Eight characters because VNC's challenge reads no more than eight - a
/// longer one would only look stronger. Six random bytes from the operating
/// system's generator (through `Uuid::new_v4`, whose first six bytes are all
/// random), base64url so it sits in a URL fragment without escaping.
fn screen_password() -> String {
    let random = uuid::Uuid::new_v4().into_bytes();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&random[..6])
}

/// `KEY=value` lines in a private temp file, for `--env-file`.
///
/// Not `-e KEY=value`: that puts the value in docker's argv, and any process on
/// this computer can read another's argv for as long as it runs. Docker reads
/// the file literally - no quoting, no expansion - so the one thing it cannot
/// carry is a line break, which is refused rather than silently split into a
/// second variable. The file is deleted when the answer is dropped.
fn env_file<'a>(
    pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<tempfile::NamedTempFile> {
    use std::io::Write;

    let failed = |e: std::io::Error| {
        ComputerError::Failed(format!("The machine's environment could not be written: {e}"))
    };
    let mut file = tempfile::NamedTempFile::new().map_err(failed)?;
    for (key, value) in pairs {
        if key.is_empty() || key.contains(['=', '\n', '\r']) || value.contains(['\n', '\r']) {
            return Err(ComputerError::Failed(format!(
                "`{key}` cannot be passed to the machine: a name must be non-empty and \
                 without `=`, and a value cannot contain a line break."
            )));
        }
        writeln!(file, "{key}={value}").map_err(failed)?;
    }
    file.flush().map_err(failed)?;
    Ok(file)
}

/// The one `docker run` a machine is started with, whether it is new or being
/// restored from a snapshot. One function so the two cannot drift: a restore
/// that ran its own shorter command used to bring a machine back with no
/// screen and none of the limits it was made with.
pub(crate) fn run_args(
    name: &str,
    from: &str,
    cpus: Option<String>,
    memory: Option<String>,
    env_file: &Path,
) -> Vec<String> {
    let workdir = image::manifest().workdir;
    let mut args: Vec<String> = vec![
        "run".into(),
        "-d".into(),
        "--name".into(),
        name.into(),
        "--label".into(),
        "inertia=1".into(),
        "-w".into(),
        workdir.clone(),
        // A network of its own. On Docker's shared default bridge every
        // container can reach every other one's ports, which made each
        // machine's screen server a door into all the others.
        "--network".into(),
        name.into(),
        // Nothing in the machine can gain privileges through a setuid binary.
        // The agent runs as an ordinary user and has no business becoming
        // root.
        "--security-opt".into(),
        "no-new-privileges".into(),
        "--pids-limit".into(),
        PIDS_LIMIT.into(),
        "--env-file".into(),
        env_file.display().to_string(),
    ];

    // Limits only when the record asked for them. Docker takes no limit at all
    // as "everything", which is the wrong default for a machine an agent drives
    // unattended - but a limit the user did not choose is worse.
    if let Some(cpus) = cpus {
        args.extend(["--cpus".into(), cpus]);
    }
    if let Some(memory) = memory {
        args.extend(["--memory".into(), memory]);
    }

    args.extend([
        // A named volume rather than a bind mount, so the machine's disk
        // survives `docker rm` and so nothing in it can reach the host's
        // filesystem through a path the user did not choose.
        "-v".into(),
        format!("{name}-workspace:{workdir}"),
        // No host port is named, so Docker picks a free one - two machines
        // running at once would collide on a fixed port. `127.0.0.1::` and not
        // `::`: the difference between "this computer can watch the screen"
        // and "the network can". A machine with a browser someone is signed
        // into does not belong on the LAN.
        "-p".into(),
        format!("127.0.0.1::{SCREEN_VIEW_PORT}"),
        "-p".into(),
        format!("127.0.0.1::{SCREEN_CONTROL_PORT}"),
        from.into(),
    ]);
    args
}

/// The CPU and memory limits a container was run with, as `docker run` takes
/// them, from `{{.HostConfig.NanoCpus}}|{{.HostConfig.Memory}}`. Zero is
/// Docker's "no limit".
pub(crate) fn parse_limits(text: &str) -> (Option<String>, Option<String>) {
    let (cpus, memory) = text.trim().split_once('|').unwrap_or_default();
    let positive = |value: &str| value.trim().parse::<u64>().ok().filter(|n| *n > 0);
    (
        positive(cpus).map(|nano| (nano as f64 / 1e9).to_string()),
        positive(memory).map(|bytes| bytes.to_string()),
    )
}

/// What a `docker` invocation produced. A non-zero code is a result here too -
/// `docker inspect` on a container that is gone is how "missing" is detected.
#[derive(Debug, Clone, Default)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// Where a streaming command's output goes as it arrives: `(stream, text)`,
/// where `stream` is `"stdout"` or `"stderr"`.
///
/// A borrowed `Fn` rather than a channel because the only caller that wants the
/// stream is the one awaiting the build, and a channel would mean a task, a
/// `'static` bound, and a second thing to shut down.
pub type Sink<'a> = &'a (dyn Fn(&str, &str) + Send + Sync);

/// Everything the image work needs from the `docker` command.
///
/// One indirection, for one reason: these tests have to be runnable on a
/// machine with no daemon, in CI, without skipping. A build whose only test is
/// "skipped, Docker not installed" is a build nobody finds out is broken until
/// a user waits fifteen minutes to find out for them.
#[async_trait]
pub trait DockerCli: Send + Sync + std::fmt::Debug {
    /// Runs and waits. A non-zero exit is a result, not an error.
    async fn run(&self, args: &[&str], timeout: Duration) -> Result<Output>;

    /// The same, handing each chunk of output to `sink` as it arrives.
    async fn stream(&self, args: &[&str], timeout: Duration, sink: Sink<'_>) -> Result<Output>;
}

/// The real `docker` on this machine.
#[derive(Debug, Default, Clone, Copy)]
pub struct Cli;

#[async_trait]
impl DockerCli for Cli {
    async fn run(&self, args: &[&str], timeout: Duration) -> Result<Output> {
        spawn_docker(args, timeout, None, None).await
    }

    async fn stream(&self, args: &[&str], timeout: Duration, sink: Sink<'_>) -> Result<Output> {
        spawn_docker(args, timeout, None, Some(sink)).await
    }
}

/// Runs `docker`, optionally feeding it stdin and optionally reporting output
/// as it arrives.
///
/// Never fails for anything the command did; only for a `docker` that could not
/// be started at all, which is a different problem with a different fix.
async fn spawn_docker(
    args: &[&str],
    timeout: Duration,
    input: Option<&str>,
    sink: Option<Sink<'_>>,
) -> Result<Output> {
    let mut command = tokio::process::Command::new("docker");
    command
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A build that timed out has to stop building. Without this the future
        // is dropped and `docker build` carries on holding the daemon.
        .kill_on_drop(true);

    #[cfg(windows)]
    {
        // Without this a console window flashes for every poll, and this polls
        // every few seconds.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command.spawn().map_err(|e| {
        ComputerError::NotReady(format!(
            "Docker could not be started: {e}. Is the Docker CLI installed and on the PATH?"
        ))
    })?;

    if let Some(text) = input {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes()).await;
            let _ = stdin.shutdown().await;
        }
    }

    // Taken before the wait, so waiting does not hold a borrow on the child
    // that the pumps also need.
    let out = child.stdout.take();
    let err = child.stderr.take();

    // Written into as the pipes are read, rather than returned at the end, so
    // that a command killed by the timeout still hands back what it managed to
    // print. It used to answer with an empty `Output`, which meant a build or a
    // test run that hung for five minutes showed nothing at all - exactly the
    // case where the last few lines are the whole story.
    let held_out = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));
    let held_err = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));

    let pump = {
        let held_out = held_out.clone();
        let held_err = held_err.clone();
        async move {
            // Both pipes at once. Reading one to the end first deadlocks the
            // moment the other fills its buffer, which for `docker build` is
            // seconds in.
            tokio::join!(
                drain(out, "stdout", sink, &held_out),
                drain(err, "stderr", sink, &held_err),
            )
        }
    };

    let both = async {
        let ((stdout, stderr), status) = tokio::join!(pump, child.wait());
        (stdout, stderr, status)
    };

    match tokio::time::timeout(timeout, both).await {
        // The child is killed by `kill_on_drop` as this future unwinds.
        Err(_) => Ok(Output {
            code: -1,
            timed_out: true,
            stdout: held_out.lock().clone(),
            stderr: held_err.lock().clone(),
        }),
        Ok((stdout, stderr, Err(e))) => {
            let _ = (stdout, stderr);
            Err(ComputerError::Failed(e.to_string()))
        }
        Ok((stdout, stderr, Ok(status))) => Ok(Output {
            code: status.code().unwrap_or(-1),
            stdout,
            stderr,
            timed_out: false,
        }),
    }
}

/// Reads one pipe to the end, reporting as it goes and accumulating the whole.
///
/// Three places, not one: the pane paints from the stream, the caller reads the
/// finished text off the answer, and `held` keeps a running copy so a command
/// the timeout kills still has something to show.
async fn drain<R>(
    pipe: Option<R>,
    stream: &str,
    sink: Option<Sink<'_>>,
    held: &parking_lot::Mutex<String>,
) -> String
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;

    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut whole = String::new();
    let mut buffer = [0u8; 8192];
    loop {
        match pipe.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                // Lossy rather than strict: a build log carries whatever a
                // compiler printed, and losing the whole stream to one bad byte
                // would be the worst possible trade.
                let text = String::from_utf8_lossy(&buffer[..read]);
                if let Some(sink) = sink {
                    sink(stream, &text);
                }
                held.lock().push_str(&text);
                whole.push_str(&text);
            }
        }
    }
    whole
}

/// Whether the image exists, and building it when it does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    /// False when it was already there. The screen says "was already built"
    /// rather than claiming credit for a no-op.
    pub built: bool,
    pub reference: String,
}

/// Long enough for a cold build: Debian, a compiler, Node and Chromium.
const BUILD_TIMEOUT: Duration = Duration::from_secs(45 * 60);

/// Is our image built, and build it if not.
///
/// Checked rather than assumed: the first provision on a new machine has no
/// image, and a `docker run` against a missing local tag tries to pull it from
/// a registry that has never heard of it, which fails with an error about
/// authentication that has nothing to do with what went wrong.
pub async fn ensure_image(cli: &dyn DockerCli, sink: Sink<'_>) -> Result<Built> {
    let manifest = image::manifest();

    let found = cli
        .run(
            &["image", "inspect", &manifest.reference],
            Duration::from_secs(15),
        )
        .await?;
    if found.code == 0 && !found.timed_out {
        return Ok(Built {
            built: false,
            reference: manifest.reference,
        });
    }

    // The context is written out of the binary at the last moment. Doing it
    // before the inspect would pay the filesystem cost on every provision, and
    // the common case is that the image is already there. Held until this
    // function returns, which is when the directory is deleted.
    let context = image::write_context().map_err(|e| {
        ComputerError::Failed(format!(
            "The sandbox build context could not be written: {e}"
        ))
    })?;

    sink(
        "stdout",
        &format!("Building {}. First time only.\n", manifest.reference),
    );

    let dir = context.path().display().to_string();
    let dockerfile = context.path().join(&manifest.dockerfile).display().to_string();
    let result = cli
        .stream(
            &[
                "build",
                "-t",
                &manifest.reference,
                "-f",
                &dockerfile,
                &dir,
            ],
            BUILD_TIMEOUT,
            sink,
        )
        .await?;

    if result.timed_out {
        return Err(ComputerError::Timeout(BUILD_TIMEOUT));
    }
    if result.code != 0 {
        // The last line, not the first: `docker build` prints the step that
        // failed at the end, and the first line of its stderr is usually the
        // buildkit banner.
        let said = last_line(&result.stderr)
            .or_else(|| last_line(&result.stdout))
            .unwrap_or_else(|| format!("docker build exited {}.", result.code));
        return Err(ComputerError::Failed(said));
    }

    Ok(Built {
        built: true,
        reference: manifest.reference,
    })
}

/// A sink that throws the output away, for the callers that only need the
/// image to exist.
pub fn quiet() -> impl Fn(&str, &str) + Send + Sync {
    |_stream: &str, _text: &str| {}
}

/// The last line with anything on it.
fn last_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rev()
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

#[derive(Debug, Default, Clone)]
pub struct DockerProvider;

impl DockerProvider {
    pub fn new() -> Self {
        Self
    }

    /// Runs `docker` with the given arguments.
    async fn run(&self, args: &[&str], timeout: Duration) -> Result<Output> {
        spawn_docker(args, timeout, None, None).await
    }

    async fn run_with_input(
        &self,
        args: &[&str],
        timeout: Duration,
        input: Option<&str>,
    ) -> Result<Output> {
        spawn_docker(args, timeout, input, None).await
    }

    /// The same, but a failure is a failure. For the operations where carrying
    /// on with a broken machine would be worse than stopping.
    async fn must(&self, args: &[&str], timeout: Duration) -> Result<String> {
        let output = self.run(args, timeout).await?;
        if output.timed_out {
            return Err(ComputerError::Timeout(timeout));
        }
        if output.code != 0 {
            return Err(ComputerError::Failed(first_line(&output.stderr).unwrap_or_else(
                || format!("docker {} failed.", args.first().copied().unwrap_or("")),
            )));
        }
        Ok(output.stdout.trim().to_string())
    }

    /// The machine's own network, made if it is not there yet.
    ///
    /// "Already exists" is success: a machine being restored, or one whose
    /// first `docker run` failed, already has one.
    async fn ensure_network(&self, name: &str) -> Result<()> {
        let output = self
            .run(
                &["network", "create", "--label", "inertia=1", name],
                Duration::from_secs(30),
            )
            .await?;
        if output.code == 0 || output.stderr.contains("already exists") {
            return Ok(());
        }
        // Docker hands each network a subnet from a fixed pool, and it runs out
        // after a few dozen. What it says then is about address pools, which
        // nobody making a computer would connect with having too many.
        if output.stderr.contains("non-overlapping") {
            return Err(ComputerError::Failed(
                "Docker has no network addresses left for another machine. Remove machines \
                 you no longer need, or unused networks with `docker network prune`."
                    .into(),
            ));
        }
        Err(ComputerError::Failed(first_line(&output.stderr).unwrap_or_else(
            || format!("The network for {name} could not be made."),
        )))
    }

    /// Runs a machine: its network, a new screen password, and the shared run
    /// arguments. Answers the container id.
    async fn launch(
        &self,
        name: &str,
        from: &str,
        cpus: Option<String>,
        memory: Option<String>,
    ) -> Result<String> {
        self.ensure_network(name).await?;

        // Held until `docker run` has read it, then deleted.
        let password = screen_password();
        let env = env_file([(SCREEN_PASSWORD_ENV, password.as_str())])?;
        let args = run_args(name, from, cpus, memory, env.path());
        let args: Vec<&str> = args.iter().map(String::as_str).collect();

        let container = self.must(&args, Duration::from_secs(120)).await?;
        Ok(container.chars().take(64).collect())
    }

    /// The container's name, which is the machine's and survives a restore
    /// that replaces the container itself.
    async fn container_name(&self, handle: &str) -> Result<String> {
        let name = self
            .must(&["inspect", "-f", "{{.Name}}", handle], Duration::from_secs(15))
            .await?;
        Ok(name.trim_start_matches('/').to_string())
    }
}

/// The first line with anything on it. Docker's errors are often a paragraph
/// whose first line is the part a person needs.
fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// Docker's state words, as the app's.
///
/// `created` and `removing` both read as stopped: neither is running and
/// neither is broken, and a machine the user is about to see should not be
/// labelled with Docker's internal vocabulary.
pub(crate) fn map_status(state: &str) -> Status {
    match state.trim() {
        "running" => Status::Running,
        "paused" => Status::Paused,
        "exited" | "created" | "removing" => Status::Stopped,
        "restarting" => Status::Provisioning,
        "dead" => Status::Error,
        _ => Status::Error,
    }
}

/// A percentage as Docker prints it: `12.34%`.
pub(crate) fn parse_percent(value: &str) -> Option<f64> {
    value.trim().trim_end_matches('%').trim().parse::<f64>().ok()
}

/// Docker's zero time, which means "never started".
pub(crate) fn started_at(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "0001-01-01T00:00:00Z" {
        return None;
    }
    Some(trimmed.to_string())
}

/// One `find -printf` line into an entry.
///
/// `find` rather than `ls`: `ls` output is for a person, its columns differ
/// between implementations, and its filenames-with-spaces are ambiguous.
/// `-printf` is exact.
pub(crate) fn parse_entry(line: &str) -> Option<DirEntry> {
    let mut parts = line.splitn(4, '\t');
    let kind = parts.next()?;
    let size = parts.next()?;
    let mtime = parts.next()?;
    // The name is whatever is left, because a filename may contain a tab.
    let name = parts.next()?.to_string();
    if name.is_empty() {
        return None;
    }

    let is_dir = kind == "d";
    Some(DirEntry {
        name,
        kind: if is_dir { "dir".into() } else { "file".into() },
        size: if is_dir { None } else { size.parse().ok() },
        modified_at: mtime
            .parse::<f64>()
            .ok()
            .filter(|seconds| *seconds > 0.0)
            .and_then(|seconds| {
                jiff::Timestamp::from_millisecond((seconds * 1000.0) as i64)
                    .ok()
                    .map(|t| t.to_string())
            }),
    })
}

/// Directories first, then names, the way a file manager orders them.
pub(crate) fn sort_entries(entries: &mut [DirEntry]) {
    entries.sort_by(|a, b| {
        (a.kind != "dir")
            .cmp(&(b.kind != "dir"))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

#[async_trait]
impl Provider for DockerProvider {
    fn id(&self) -> &'static str {
        "docker"
    }

    fn label(&self) -> &'static str {
        "Docker"
    }

    fn blurb(&self) -> &'static str {
        "A container on this computer. Nothing leaves the machine, and the disk survives a restart."
    }

    /// Whether Docker can be used right now.
    ///
    /// `docker version` rather than `docker --version`: the second answers from
    /// the CLI alone and says yes while the daemon is stopped, which is exactly
    /// the state a person needs to be told about.
    async fn available(&self) -> Readiness {
        match self.run(&["version", "--format", "{{.Server.Version}}"], Duration::from_secs(10)).await {
            Err(ComputerError::NotReady(reason)) => Readiness::not(reason),
            Err(e) => Readiness::not(e.to_string()),
            Ok(output) if output.timed_out => {
                Readiness::not("Docker did not answer. It may be starting up.")
            }
            Ok(output) if output.code == 0 && !output.stdout.trim().is_empty() => Readiness::ready(),
            Ok(output) => Readiness::not(
                first_line(&output.stderr)
                    .unwrap_or_else(|| "Docker is installed but its daemon is not running.".into()),
            ),
        }
    }

    async fn create(&self, spec: &Spec) -> Result<Created> {
        let manifest = image::manifest();

        // The first machine on a new install builds the image. It is why the
        // settings screen offers a Build button - fifteen minutes inside "make
        // me a computer" reads as a hang - but a person who skipped that button
        // still has to get a machine rather than an error about a tag that
        // cannot be pulled.
        ensure_image(&Cli, &quiet()).await?;

        let name = machine_name(&spec.id);

        // A container left over from a machine with this id - a crash between
        // the run and the record being written - would make `docker run` fail
        // on a name conflict, which reads as a bug in the app rather than as
        // debris.
        let _ = self.run(&["rm", "-f", &name], Duration::from_secs(20)).await;

        let cpus = spec.cpu.filter(|c| *c > 0.0).map(|c| c.to_string());
        let memory = spec.memory_gb.filter(|m| *m > 0.0).map(|m| format!("{m}g"));
        let container = self.launch(&name, &manifest.reference, cpus, memory).await?;

        Ok(Created {
            handle: container,
            name,
            status: Status::Running,
            os: "Debian 12 (container)".into(),
            image: manifest.reference,
            workdir: manifest.workdir,
        })
    }

    async fn start(&self, handle: &str) -> Result<()> {
        self.must(&["start", handle], Duration::from_secs(60)).await?;
        Ok(())
    }

    async fn stop(&self, handle: &str) -> Result<()> {
        // Not `must`: stopping something already stopped is the outcome the
        // caller wanted, and reporting it as a failure makes a Stop button
        // that errors on a stopped machine.
        self.run(&["stop", "-t", "10", handle], Duration::from_secs(60)).await?;
        Ok(())
    }

    async fn pause(&self, handle: &str) -> Result<()> {
        self.must(&["pause", handle], Duration::from_secs(30)).await?;
        Ok(())
    }

    async fn resume(&self, handle: &str) -> Result<()> {
        self.must(&["unpause", handle], Duration::from_secs(30)).await?;
        Ok(())
    }

    async fn remove(&self, handle: &str, name: Option<&str>) -> Result<()> {
        self.run(&["rm", "-f", handle], Duration::from_secs(60)).await?;
        // The volume and the network go with it, or a removed machine leaves
        // them behind forever with nothing in the UI that could ever mention
        // them again. The network's removal is allowed to fail: a machine made
        // before machines had networks of their own has none.
        if let Some(name) = name {
            let volume = format!("{name}-workspace");
            let _ = self
                .run(&["volume", "rm", "-f", &volume], Duration::from_secs(30))
                .await;
            let _ = self
                .run(&["network", "rm", name], Duration::from_secs(30))
                .await;
        }
        Ok(())
    }

    async fn status(&self, handle: &str) -> Health {
        let output = self
            .run(
                &["inspect", "-f", "{{.State.Status}}|{{.State.StartedAt}}", handle],
                Duration::from_secs(15),
            )
            .await;

        // A container that is gone is an ordinary state of the world, not a
        // failure: the user can remove one by hand at any moment.
        let Ok(output) = output else {
            return Health {
                status: Status::Missing,
                started_at: None,
            };
        };
        if output.code != 0 || output.timed_out {
            return Health {
                status: Status::Missing,
                started_at: None,
            };
        }

        let text = output.stdout.trim();
        let (state, started) = text.split_once('|').unwrap_or((text, ""));
        Health {
            status: map_status(state),
            started_at: started_at(started),
        }
    }

    async fn stats(&self, handle: &str) -> Stats {
        let output = self
            .run(
                &[
                    "stats",
                    "--no-stream",
                    "--format",
                    "{{.CPUPerc}}|{{.MemPerc}}",
                    handle,
                ],
                Duration::from_secs(20),
            )
            .await;

        let mut stats = Stats {
            scope: "container".into(),
            ..Default::default()
        };
        let Ok(output) = output else { return stats };
        if output.code != 0 {
            return stats;
        }

        let text = output.stdout.trim();
        let (cpu, mem) = text.split_once('|').unwrap_or((text, ""));
        stats.cpu_pct = parse_percent(cpu);
        stats.mem_pct = parse_percent(mem);
        stats
    }

    async fn exec(&self, handle: &str, request: &ExecRequest) -> Result<ExecResult> {
        let manifest = image::manifest();
        let line = request.command.trim();
        if line.is_empty() {
            return Ok(ExecResult::default());
        }

        let timeout = request.timeout.unwrap_or(DEFAULT_TIMEOUT);

        // Through a file rather than `-e`, because a caller's environment is
        // where a token goes, and argv is readable by the whole host. Held
        // until the command returns.
        let env = if request.env.is_empty() {
            None
        } else {
            Some(env_file(
                request.env.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            )?)
        };
        let env_path = env.as_ref().map(|file| file.path().display().to_string());

        let mut args: Vec<&str> = vec!["exec"];
        if let Some(cwd) = request.cwd.as_deref().filter(|c| !c.is_empty()) {
            args.extend(["-w", cwd]);
        }
        if let Some(path) = &env_path {
            args.extend(["--env-file", path]);
        }
        args.push(handle);
        for part in &manifest.shell {
            args.push(part);
        }
        args.push(line);

        let started = Instant::now();
        let output = self.run(&args, timeout).await?;

        Ok(ExecResult {
            code: output.code,
            stdout: output.stdout,
            stderr: if output.timed_out {
                format!("{}\nTimed out after {timeout:?}.", output.stderr)
            } else {
                output.stderr
            },
            duration_ms: started.elapsed().as_millis() as u64,
            timed_out: output.timed_out,
        })
    }

    async fn list_dir(&self, handle: &str, path: &str) -> Result<Vec<DirEntry>> {
        let target = if path.trim().is_empty() { "." } else { path };
        let command = format!(
            r"find {} -mindepth 1 -maxdepth 1 -printf '%y\t%s\t%T@\t%f\n' 2>/dev/null | head -c 200000",
            shell_quote(target)
        );

        let result = self
            .exec(
                handle,
                &ExecRequest {
                    command,
                    timeout: Some(Duration::from_secs(30)),
                    ..Default::default()
                },
            )
            .await?;

        if !result.ok() && result.stdout.is_empty() {
            return Err(ComputerError::Failed(
                first_line(&result.stderr).unwrap_or_else(|| format!("Cannot read {target}")),
            ));
        }

        let mut entries: Vec<DirEntry> = result.stdout.lines().filter_map(parse_entry).collect();
        sort_entries(&mut entries);
        Ok(entries)
    }

    async fn read_file(&self, handle: &str, path: &str) -> Result<String> {
        let result = self
            .exec(
                handle,
                &ExecRequest {
                    command: format!("cat {}", shell_quote(path)),
                    timeout: Some(Duration::from_secs(30)),
                    ..Default::default()
                },
            )
            .await?;
        if !result.ok() {
            return Err(ComputerError::Failed(
                first_line(&result.stderr).unwrap_or_else(|| format!("Cannot read {path}")),
            ));
        }
        Ok(result.stdout)
    }

    /// The file as base64, for an image or a download - bytes, not text.
    async fn read_file_base64(&self, handle: &str, path: &str) -> Result<String> {
        let result = self
            .exec(
                handle,
                &ExecRequest {
                    command: format!("base64 -w0 {}", shell_quote(path)),
                    timeout: Some(Duration::from_secs(60)),
                    ..Default::default()
                },
            )
            .await?;
        if !result.ok() {
            return Err(ComputerError::Failed(
                first_line(&result.stderr).unwrap_or_else(|| format!("Cannot read {path}")),
            ));
        }
        // `-w0` should already be one line; strip any stray whitespace a
        // busybox base64 adds so the string decodes cleanly.
        Ok(result.stdout.split_whitespace().collect())
    }

    /// Writes a file by piping it in, base64 encoded.
    ///
    /// Not `echo`: the content is arbitrary and would have to survive shell
    /// quoting, a heredoc delimiter that might appear in the text, and whatever
    /// the locale does to a byte that is not valid UTF-8. base64 has none of
    /// those questions.
    async fn write_file(&self, handle: &str, path: &str, content: &str) -> Result<()> {
        let manifest = image::manifest();
        let encoded = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
        let quoted = shell_quote(path);
        let command = format!("mkdir -p \"$(dirname {quoted})\" && base64 -d > {quoted}");

        let mut args: Vec<&str> = vec!["exec", "-i", handle];
        for part in &manifest.shell {
            args.push(part);
        }
        args.push(&command);

        let output = self
            .run_with_input(&args, Duration::from_secs(60), Some(&encoded))
            .await?;
        if output.code != 0 {
            return Err(ComputerError::Failed(
                first_line(&output.stderr).unwrap_or_else(|| format!("Cannot write {path}")),
            ));
        }
        Ok(())
    }

    async fn snapshot(&self, handle: &str, name: &str) -> Result<Snapshot> {
        let id = format!("snap-{}", jiff::Timestamp::now().as_millisecond());
        let tag = format!("inertia-snapshot:{id}");
        let owner = format!("LABEL {MACHINE_LABEL}={}", self.container_name(handle).await?);
        self.must(
            &["commit", "-m", name, "-c", &owner, handle, &tag],
            Duration::from_secs(300),
        )
        .await?;

        Ok(Snapshot {
            id,
            name: name.to_string(),
            created_at: jiff::Timestamp::now().to_string(),
            size: None,
        })
    }

    /// This machine's snapshots, and no other's. A machine whose container is
    /// gone has no name to look them up by, and lists none.
    async fn snapshots(&self, handle: &str) -> Result<Vec<Snapshot>> {
        let Ok(name) = self.container_name(handle).await else {
            return Ok(Vec::new());
        };
        let owned = format!("label={MACHINE_LABEL}={name}");
        let output = self
            .run(
                &[
                    "images",
                    "inertia-snapshot",
                    "--filter",
                    &owned,
                    "--format",
                    "{{.Tag}}|{{.CreatedAt}}|{{.Size}}",
                ],
                Duration::from_secs(30),
            )
            .await?;
        if output.code != 0 {
            return Ok(Vec::new());
        }

        Ok(output
            .stdout
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '|');
                let id = parts.next()?.trim().to_string();
                if id.is_empty() {
                    return None;
                }
                Some(Snapshot {
                    name: id.clone(),
                    id,
                    created_at: parts.next().unwrap_or_default().trim().to_string(),
                    size: None,
                })
            })
            .collect())
    }

    async fn restore(&self, handle: &str, snapshot_id: &str, name: &str) -> Result<Created> {
        let manifest = image::manifest();
        let tag = format!("inertia-snapshot:{snapshot_id}");

        // Checked before anything is destroyed: restoring onto a snapshot that
        // is not there would remove the running container and leave nothing to
        // put back. And it has to be this machine's own - an id is only a
        // timestamp, and another machine's disk is not this one's past.
        let owner_of = format!("{{{{index .Config.Labels \"{MACHINE_LABEL}\"}}}}");
        let owner = self
            .must(&["image", "inspect", "-f", &owner_of, &tag], Duration::from_secs(15))
            .await
            .unwrap_or_default();
        if owner != name {
            return Err(ComputerError::Failed(format!(
                "This machine has no snapshot called {snapshot_id}."
            )));
        }

        // The limits it was made with, read before the container they are on
        // goes. A container that is already gone had none worth keeping.
        let limits = self
            .must(
                &["inspect", "-f", "{{.HostConfig.NanoCpus}}|{{.HostConfig.Memory}}", handle],
                Duration::from_secs(15),
            )
            .await
            .unwrap_or_default();
        let (cpus, memory) = parse_limits(&limits);

        let _ = self.run(&["rm", "-f", handle], Duration::from_secs(60)).await;
        let container = self.launch(name, &tag, cpus, memory).await?;

        Ok(Created {
            handle: container,
            name: name.to_string(),
            status: Status::Running,
            os: "Debian 12 (container)".into(),
            image: tag,
            workdir: manifest.workdir,
        })
    }

    /// A PNG data URL, or `None` when the machine has no display.
    ///
    /// A container built without X has no screen, and that is not a failure -
    /// the pane draws "no display" from the `None`.
    async fn screenshot(&self, handle: &str) -> Result<Option<String>> {
        let result = self
            .exec(
                handle,
                &ExecRequest {
                    command: crate::desktop::still(),
                    timeout: Some(Duration::from_secs(30)),
                    ..Default::default()
                },
            )
            .await?;

        let encoded: String = result.stdout.split_whitespace().collect();
        if !result.ok() || encoded.is_empty() {
            return Ok(None);
        }
        Ok(Some(format!("data:image/png;base64,{encoded}")))
    }

    /// The host ports the two noVNC servers landed on, as URLs a browser can
    /// open.
    ///
    /// `None` unless both are there. A container from before the image had a
    /// VNC server in it, or one whose publish failed, has no mapping - and half
    /// a screen is not worth showing, since the "take control" switch would
    /// then be a button that does nothing.
    ///
    /// The password rides in the URL's fragment, which noVNC reads as
    /// `password` and the browser never sends to the server, so it is in no
    /// request line and no websockify log. It is read back off the container
    /// rather than kept on the machine's record: the container already holds
    /// it, and a record is a file in the workspace that gets copied, synced and
    /// committed. A machine from before screens had passwords has none, and
    /// its URL has no fragment - its screen server never asks for one.
    async fn screen(&self, handle: &str) -> Result<Option<Screen>> {
        let env = self
            .run(
                &["inspect", "-f", "{{range .Config.Env}}{{println .}}{{end}}", handle],
                Duration::from_secs(10),
            )
            .await?;
        let fragment = screen_fragment(&env.stdout);

        let mut found = Vec::new();
        for port in [SCREEN_VIEW_PORT, SCREEN_CONTROL_PORT] {
            let asked = port.to_string();
            let result = self
                .run(&["port", handle, &asked], Duration::from_secs(10))
                .await?;
            if result.code != 0 || result.timed_out {
                return Ok(None);
            }
            match published_port(&result.stdout) {
                Some(mapped) => found.push(mapped),
                None => return Ok(None),
            }
        }

        Ok(Some(Screen {
            view: format!("http://127.0.0.1:{}/vnc.html{fragment}", found[0]),
            control: format!("http://127.0.0.1:{}/vnc.html{fragment}", found[1]),
        }))
    }
}

/// `#password=...` for a container whose environment carries a screen
/// password, or nothing for one that does not. `env` is one `KEY=value` per
/// line, as `docker inspect` prints `.Config.Env`.
pub(crate) fn screen_fragment(env: &str) -> String {
    let prefix = format!("{SCREEN_PASSWORD_ENV}=");
    env.lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .filter(|password| !password.is_empty())
        .map(|password| format!("#password={password}"))
        .unwrap_or_default()
}

/// The host port out of `docker port`'s answer.
///
/// One line per binding, each `0.0.0.0:49154` or `[::]:49154`, so the number
/// after the last colon on the first usable line is the one. Parsed rather than
/// assumed: a container publishing on both IPv4 and IPv6 prints two lines and
/// they can differ.
pub(crate) fn published_port(stdout: &str) -> Option<u16> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .find_map(|line| line.rsplit(':').next()?.parse::<u16>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A `docker` that never ran, with a scripted answer per subcommand and a
    /// stdout it hands over in pieces.
    ///
    /// The pieces matter: a build's value to the person watching is that the
    /// log appears while it happens, and a fake that delivered it in one lump
    /// would pass a test that the real thing fails.
    #[derive(Debug)]
    struct FakeCli {
        /// Exit code for `docker image inspect`. Non-zero means "not built".
        inspect: i32,
        /// Exit code for `docker build`.
        build: i32,
        /// What the build prints, chunk by chunk.
        chunks: Vec<(&'static str, &'static str)>,
        calls: Mutex<Vec<Vec<String>>>,
        /// Whether the Dockerfile the build was pointed at was a real file
        /// while the build ran. Checked then, because the directory is gone
        /// by the time the call returns.
        dockerfile_was_there: Mutex<bool>,
    }

    impl FakeCli {
        fn new(inspect: i32, build: i32, chunks: Vec<(&'static str, &'static str)>) -> Self {
            Self {
                inspect,
                build,
                chunks,
                calls: Mutex::new(Vec::new()),
                dockerfile_was_there: Mutex::new(false),
            }
        }

        fn record(&self, args: &[&str]) {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|a| (*a).to_string()).collect());
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl DockerCli for FakeCli {
        async fn run(&self, args: &[&str], _timeout: Duration) -> Result<Output> {
            self.record(args);
            Ok(Output {
                code: self.inspect,
                stderr: if self.inspect == 0 {
                    String::new()
                } else {
                    "Error: No such image".into()
                },
                ..Default::default()
            })
        }

        async fn stream(&self, args: &[&str], _timeout: Duration, sink: Sink<'_>) -> Result<Output> {
            self.record(args);
            *self.dockerfile_was_there.lock().unwrap() = Path::new(args[4]).is_file();
            let mut stdout = String::new();
            let mut stderr = String::new();
            for (stream, text) in &self.chunks {
                sink(stream, text);
                if *stream == "stderr" {
                    stderr.push_str(text);
                } else {
                    stdout.push_str(text);
                }
            }
            Ok(Output {
                code: self.build,
                stdout,
                stderr,
                timed_out: false,
            })
        }
    }

    /// What a build reported, in order: `(stream, text)`.
    type Reported = std::sync::Arc<Mutex<Vec<(String, String)>>>;

    /// Collects what a build reported, in order.
    fn recorder() -> (Reported, impl Fn(&str, &str) + Send + Sync) {
        let seen: Reported = std::sync::Arc::new(Mutex::new(Vec::new()));
        let held = seen.clone();
        (seen, move |stream: &str, text: &str| {
            held.lock().unwrap().push((stream.to_string(), text.to_string()));
        })
    }

    /// An image that is already there is not rebuilt. Rebuilding it would put
    /// fifteen minutes in front of a person who pressed a button expecting
    /// nothing to happen.
    #[tokio::test]
    async fn an_image_that_exists_is_not_built_again() {
        let cli = FakeCli::new(0, 0, Vec::new());
        let (seen, sink) = recorder();
        let built = ensure_image(&cli, &sink).await.unwrap();

        assert!(!built.built);
        assert_eq!(built.reference, image::manifest().reference);
        assert_eq!(cli.calls().len(), 1, "the build must not have been reached");
        assert!(seen.lock().unwrap().is_empty());
    }

    /// The build streams as it goes, and it builds the embedded context.
    #[tokio::test]
    async fn a_missing_image_is_built_from_the_embedded_context_and_streams() {
        let cli = FakeCli::new(1, 0, vec![
            ("stdout", "Step 1/9 : FROM debian:bookworm-slim
"),
            ("stderr", "#4 resolve docker.io/library/debian
"),
            ("stdout", "Successfully tagged inertia-sandbox:1.1.0
"),
        ]);
        let (seen, sink) = recorder();
        let built = ensure_image(&cli, &sink).await.unwrap();

        assert!(built.built);

        let calls = cli.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0][..2], ["image".to_string(), "inspect".to_string()]);

        let build = &calls[1];
        assert_eq!(build[0], "build");
        assert_eq!(build[1], "-t");
        assert_eq!(build[2], image::manifest().reference);
        assert_eq!(build[3], "-f");
        // The Dockerfile it was pointed at was a real file, written out of the
        // binary by this call. That is the whole claim of embedding it.
        assert!(*cli.dockerfile_was_there.lock().unwrap());
        // And the context is gone once the build is: it is a private
        // directory per build, not a shared one left in temp.
        assert!(!Path::new(&build[5]).exists());

        let reported = seen.lock().unwrap().clone();
        // The first line says what is about to happen, because a person who
        // pressed Build and sees nothing for a minute assumes it hung.
        assert!(reported[0].1.contains(&format!("Building {}", image::manifest().reference)));
        assert_eq!(reported[1].0, "stdout");
        assert!(reported[1].1.contains("FROM debian"));
        // stderr is carried through as stderr: buildkit writes its progress
        // there, and folding it into stdout would lose which is which.
        assert_eq!(reported[2].0, "stderr");
        assert_eq!(reported.len(), 4);
    }

    /// A failed build reports the step that failed, which `docker build` prints
    /// last. The first line is the buildkit banner and says nothing.
    #[tokio::test]
    async fn a_failed_build_reports_the_line_that_says_why() {
        let cli = FakeCli::new(1, 1, vec![(
            "stderr",
            "#1 [internal] load build definition
ERROR: failed to solve: chromium: not found
",
        )]);
        let (_seen, sink) = recorder();
        let failure = ensure_image(&cli, &sink).await.unwrap_err();
        assert!(failure.to_string().contains("chromium: not found"), "{failure}");
    }


    /// What a new machine and a restored one are both run with.
    #[test]
    fn a_machine_runs_isolated_hardened_and_published_to_loopback_only() {
        let env = Path::new("/tmp/env");
        let args = run_args("inertia-box", "inertia-sandbox:1.1.0", None, None, env);
        let pair = |flag: &str, value: &str| {
            args.windows(2).any(|w| w[0] == flag && w[1] == value)
        };

        assert_eq!(args[..2], ["run".to_string(), "-d".to_string()]);
        assert!(pair("--name", "inertia-box"));
        // Its own network, not the shared bridge every container can see.
        assert!(pair("--network", "inertia-box"));
        assert!(pair("--security-opt", "no-new-privileges"));
        assert!(pair("--pids-limit", PIDS_LIMIT));
        // The password arrives through a file, never as a value in argv.
        assert!(pair("--env-file", &env.display().to_string()));
        assert!(!args.iter().any(|a| a == "-e" || a.contains(SCREEN_PASSWORD_ENV)));
        assert!(pair("-v", "inertia-box-workspace:/workspace"));
        assert!(pair("-p", "127.0.0.1::6080"));
        assert!(pair("-p", "127.0.0.1::6081"));
        assert_eq!(args.last().unwrap(), "inertia-sandbox:1.1.0");
        // No limit unless one was asked for.
        assert!(!args.iter().any(|a| a == "--cpus" || a == "--memory"));

        let limited = run_args("inertia-box", "x", Some("2".into()), Some("8g".into()), env);
        assert!(limited.windows(2).any(|w| w[0] == "--cpus" && w[1] == "2"));
        assert!(limited.windows(2).any(|w| w[0] == "--memory" && w[1] == "8g"));
    }

    /// A restore runs with the limits the machine had, read off its container.
    #[test]
    fn limits_come_back_off_docker_inspect() {
        assert_eq!(
            parse_limits("2500000000|8589934592\n"),
            (Some("2.5".into()), Some("8589934592".into()))
        );
        assert_eq!(parse_limits("0|0"), (None, None));
        assert_eq!(parse_limits(""), (None, None));
    }

    #[test]
    fn each_screen_password_is_new_and_url_safe() {
        let one = screen_password();
        let two = screen_password();
        assert_eq!(one.len(), 8);
        assert_ne!(one, two);
        assert!(one
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn the_env_file_holds_each_pair_and_refuses_a_line_break() {
        let file = env_file([("A", "one two"), ("B", "x=y")]).unwrap();
        assert_eq!(std::fs::read_to_string(file.path()).unwrap(), "A=one two\nB=x=y\n");

        // A line break would end the value and start a variable nobody asked
        // for, so it is refused rather than written.
        assert!(env_file([("A", "one\nEVIL=1")]).is_err());
        assert!(env_file([("A=B", "x")]).is_err());
        assert!(env_file([("", "x")]).is_err());
    }

    #[test]
    fn the_screen_url_carries_the_password_only_when_the_container_has_one() {
        assert_eq!(
            screen_fragment("PATH=/usr/bin\nINERTIA_SCREEN_PASSWORD=Ab-_12xY\nHOME=/home/agent\n"),
            "#password=Ab-_12xY"
        );
        // A machine from before screens had passwords.
        assert_eq!(screen_fragment("PATH=/usr/bin\n"), "");
        assert_eq!(screen_fragment("INERTIA_SCREEN_PASSWORD=\n"), "");
    }

    /// `docker port` prints one line per binding, and a container published
    /// on both IPv4 and IPv6 prints two that can differ.
    #[test]
    fn the_host_port_is_read_off_dockers_answer() {
        assert_eq!(published_port("0.0.0.0:49154
"), Some(49154));
        assert_eq!(published_port("127.0.0.1:32770"), Some(32770));
        assert_eq!(published_port("[::]:49155
0.0.0.0:49154
"), Some(49155));
        // A container with no such publish prints nothing, and half a screen is
        // not worth showing.
        assert_eq!(published_port(""), None);
        assert_eq!(published_port("not a mapping"), None);
    }

    #[test]
    fn docker_state_words_become_the_apps() {
        assert_eq!(map_status("running"), Status::Running);
        assert_eq!(map_status("paused"), Status::Paused);
        assert_eq!(map_status("exited"), Status::Stopped);
        assert_eq!(map_status("restarting"), Status::Provisioning);
        assert_eq!(map_status("dead"), Status::Error);
    }

    /// Neither is running and neither is broken, so neither should be shown
    /// with Docker's internal vocabulary.
    #[test]
    fn created_and_removing_both_read_as_stopped() {
        assert_eq!(map_status("created"), Status::Stopped);
        assert_eq!(map_status("removing"), Status::Stopped);
    }

    #[test]
    fn an_unknown_state_is_an_error_not_a_panic() {
        assert_eq!(map_status("something-new"), Status::Error);
    }

    #[test]
    fn percentages_come_off_dockers_strings() {
        assert_eq!(parse_percent("12.34%"), Some(12.34));
        assert_eq!(parse_percent(" 0.00% "), Some(0.0));
        assert_eq!(parse_percent("--"), None);
        assert_eq!(parse_percent(""), None);
    }

    /// Docker prints a zero time for a container that has never run, and
    /// showing it as a date would be a lie about when it started.
    #[test]
    fn the_zero_time_is_never_started() {
        assert_eq!(started_at("0001-01-01T00:00:00Z"), None);
        assert_eq!(started_at("  "), None);
        assert_eq!(
            started_at("2026-09-07T16:52:59Z").as_deref(),
            Some("2026-09-07T16:52:59Z")
        );
    }

    #[test]
    fn a_find_line_becomes_an_entry() {
        let entry = parse_entry("f\t1024\t1757260379\tnotes.md").unwrap();
        assert_eq!(entry.name, "notes.md");
        assert_eq!(entry.kind, "file");
        assert_eq!(entry.size, Some(1024));
        assert!(entry.modified_at.is_some());
    }

    /// A directory's size is meaningless here - `find` reports the size of the
    /// directory entry, not of what is in it.
    #[test]
    fn a_directory_reports_no_size() {
        let entry = parse_entry("d\t4096\t1757260379\tsrc").unwrap();
        assert_eq!(entry.kind, "dir");
        assert_eq!(entry.size, None);
    }

    /// The whole reason for `find -printf` over `ls`.
    #[test]
    fn a_filename_containing_a_tab_survives() {
        let entry = parse_entry("f\t10\t1757260379\todd\tname.txt").unwrap();
        assert_eq!(entry.name, "odd\tname.txt");
    }

    #[test]
    fn a_malformed_line_is_skipped_rather_than_guessed_at() {
        assert!(parse_entry("").is_none());
        assert!(parse_entry("f\t10").is_none());
        assert!(parse_entry("f\t10\t123\t").is_none());
    }

    #[test]
    fn listings_put_directories_first_then_names() {
        let mut entries: Vec<DirEntry> = ["f\t1\t1\tzebra.txt", "d\t1\t1\tsrc", "f\t1\t1\tApple.txt"]
            .iter()
            .filter_map(|line| parse_entry(line))
            .collect();
        sort_entries(&mut entries);

        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["src", "Apple.txt", "zebra.txt"]);
    }

    #[test]
    fn the_first_useful_line_is_what_a_person_is_shown() {
        assert_eq!(
            first_line("\n\nError: no such container\nmore detail"),
            Some("Error: no such container".to_string())
        );
        assert_eq!(first_line("   "), None);
    }
}
