//! Driving a machine's screen.
//!
//! `exec` lets an agent run a command. That is enough for a build and useless
//! for a web page: what an agent most often needs from a computer is to open
//! something, look at it, and click. This is the other half.
//!
//! ## The shape, and why it is this shape
//!
//! An action is data, not a command. [`Action::Pointer`] is compiled to an
//! xdotool line here, at the last moment, which is what lets a whole
//! interaction arrive as one list and run as one exec: click the field, type
//! the address, press Return, wait, look. Five things, one round trip.
//!
//! The alternative - a tool call per action - is what this replaces. It cost
//! five round trips through the model for a login, each one carrying a full
//! screenshot back, and by the time the password was typed the conversation was
//! mostly pictures of a form being filled in.
//!
//! Everything goes through xdotool and `import` against the machine's own
//! display, so it works identically over a Docker exec and a cloud provider's
//! HTTP call: no port to open, no VNC, no protocol, no second transport to keep
//! alive. A machine with no display degrades to a clear "there is no screen
//! here" rather than to a connection that hangs.
//!
//! ## Three rules the commands here follow
//!
//! **Every command states what it needs.** A machine built from a plain base
//! image has no xdotool and no X server, and the honest failure is a sentence
//! saying so, not a 127 with "not found" on stderr.
//!
//! **Nothing is matched by `pkill -f`.** The pattern that matches a browser
//! also matches the shell that launched it - its own arguments contain the
//! string - so a launcher written that way kills itself before it launches
//! anything.
//!
//! **Typing is a paste.** See [`action_command`].

use base64::Engine;
use serde_json::{Map, Value};

/// Prefixed to every command, so nothing has to remember the display.
pub const WITH_DISPLAY: &str = r#"export DISPLAY="${DISPLAY:-:99}";"#;

/// The exit code a machine uses to say "I cannot do that", as opposed to "that
/// did not work". Raised to the model as a refusal rather than as output: one
/// handed "no display" as ordinary text will try clicking anyway.
pub const NO_DESKTOP_CODE: i32 = 3;

/// Refuses with a sentence rather than a 127, and names the cause.
pub const NEEDS_DISPLAY: &str = concat!(
    "if ! command -v xdotool >/dev/null 2>&1; then ",
    r#"echo "This machine has no desktop tools. It was built from a plain base image rather than the Inertia sandbox image."; exit 3; fi; "#,
    r#"if ! xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then "#,
    r#"echo "This machine has no display running."; exit 3; fi;"#,
);

/// The most actions one batch may carry.
pub const MAX_ACTIONS: usize = 24;

/// The longest a batch may be told to wait for the screen to settle.
pub const MAX_SETTLE_MS: i64 = 5_000;

/// The longest an explicit `wait` action may pause for.
///
/// Separate from the settle above, and much longer, because they are different
/// things: the settle is "let the click land before photographing it", and a
/// wait is "this page is generating an image". Sharing one ceiling meant a
/// model that asked to wait fifteen seconds was silently given five - and it
/// could not tell, so it looked, saw the same spinner, and asked again. Three
/// rounds of that in a real session, each carrying a full screenshot back, to
/// cover the fifteen seconds it had asked for the first time.
pub const MAX_WAIT_MS: i64 = 30_000;

/// The keys that are only ever held down, so anything else in the list is the
/// key itself.
const MODIFIERS: &[&str] = &[
    "ctrl", "control", "alt", "shift", "super", "meta", "cmd", "command",
];

/// The mouse buttons a model may name, for the same forgiving read as above.
const BUTTONS: &[&str] = &["left", "right", "middle"];

/// The kinds a model may name.
pub const KINDS: &[&str] = &[
    "click", "move", "down", "up", "type", "key", "scroll", "wait",
];

/// The fields an action may carry, so stray ones beside the list can be folded
/// back in.
const ACTION_FIELDS: &[&str] = &[
    "kind",
    "x",
    "y",
    "button",
    "double",
    "text",
    "key",
    "modifiers",
    "direction",
    "amount",
    "ms",
];

/// What a correct call looks like, quoted in every complaint about a wrong one.
pub const EXAMPLE: &str = r#"{"actions":[{"kind":"click","x":640,"y":360},{"kind":"type","text":"hello"},{"kind":"key","key":"Return"}]}"#;

/// A single-quoted shell string, safe for anything a model might send.
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// A number out of whatever the model wrote, rounded. Strings count: a model
/// that sends `"640"` where 640 was asked for means 640, and refusing it costs
/// a round trip to learn nothing.
fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn bounded(value: Option<&Value>, min: i64, max: i64, fallback: i64) -> i64 {
    match number(value) {
        Some(n) if n.is_finite() => (n.round() as i64).clamp(min, max),
        _ => fallback,
    }
}

fn coordinate(value: Option<&Value>, name: &str) -> Result<i64, String> {
    match number(value) {
        Some(n) if n.is_finite() && (0.0..=100_000.0).contains(&n) => Ok(n.round() as i64),
        _ => Err(format!(
            "computer action {name} must be a non-negative coordinate"
        )),
    }
}

fn is_modifier(name: &str) -> bool {
    MODIFIERS.contains(&name.trim().to_lowercase().as_str())
}

/// The action list as the model actually sent it, lifted into a list of
/// objects.
///
/// Measured, every one of these. A model that has just been told the schema
/// still sends the batch as one object rather than a list of one; sends the
/// kind as a bare string - `"click"`, and once `"kind>click"` - with the
/// coordinates beside the list instead of inside it; or sends the fields of a
/// single action at the top level and no list at all. None of those is
/// ambiguous. Each was refused with a sentence about the shape, and the model
/// answered every refusal with the same shape again, and the turn ended with a
/// button never pressed. Reading what was clearly meant costs nothing and
/// refusing it cost the whole task.
///
/// `extra` is the rest of the call: whatever the model wrote beside `actions`.
/// Its action fields fill in what an item lacks, so `{"actions":["click"],
/// "x":10,"y":20}` is a click at 10,20.
pub fn lift_actions(value: Option<&Value>, extra: &Value) -> Result<Vec<Value>, String> {
    let mut spare = Map::new();
    for field in ACTION_FIELDS {
        if let Some(found) = extra.get(*field) {
            if !found.is_null() {
                spare.insert((*field).to_string(), found.clone());
            }
        }
    }

    // A list inside the list means one action, wrapped twice. It is what a
    // model whose tool calls are written as markup produces when the item
    // element survives the conversion to JSON, and it is never anything else
    // here.
    let list: Vec<Value> = match value {
        None | Some(Value::Null) => {
            if spare.contains_key("kind") {
                vec![Value::Object(Map::new())]
            } else {
                Vec::new()
            }
        }
        Some(Value::String(s)) if s.is_empty() => {
            if spare.contains_key("kind") {
                vec![Value::Object(Map::new())]
            } else {
                Vec::new()
            }
        }
        Some(Value::Array(items)) => flatten(items, 2),
        Some(other) => vec![other.clone()],
    };

    let mut lifted: Vec<Value> = Vec::new();
    for raw in list {
        let mut item = match raw {
            Value::Object(map) => Value::Object(map),
            Value::String(text) => {
                // A bare button name, tacked on after the action it belongs to.
                //
                // Measured: `{"actions":[{"kind":"click","x":507,"y":527},
                // "right"]}` is what a model writes when it remembers that a
                // right click needs the word "right" and not where the word
                // goes. There is nothing else it could mean, and refusing it
                // cost a turn and a user typing "?" to find out why nothing
                // happened. Folded into the pointer action before it.
                let lowered = text.to_lowercase();
                let button = BUTTONS.contains(&lowered.as_str());
                if button {
                    if let Some(previous) = lifted.last_mut() {
                        if previous.get("button").is_none() {
                            previous["button"] = Value::String(lowered);
                            continue;
                        }
                    }
                }
                // The kind, somewhere in the string. "click", "kind>click",
                // "kind: click" all name one action and nothing else a string
                // could be.
                let word = KINDS.iter().find(|kind| word_in(&lowered, kind));
                let mut map = Map::new();
                map.insert(
                    "kind".into(),
                    Value::String(word.map_or(text, |found| (*found).to_string())),
                );
                Value::Object(map)
            }
            other => {
                return Err(format!(
                    "Each action must be an object, like {EXAMPLE}. You sent {other}."
                ))
            }
        };

        for (field, fallback) in &spare {
            let missing = item.get(field).is_none_or(Value::is_null);
            if missing {
                item[field] = fallback.clone();
            }
        }
        lifted.push(item);
    }
    Ok(lifted)
}

/// Whether `word` appears in `text` on its own, the way `\bclick\b` does.
fn word_in(text: &str, word: &str) -> bool {
    let boundary = |c: Option<char>| !c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut from = 0;
    while let Some(at) = text[from..].find(word) {
        let start = from + at;
        let end = start + word.len();
        if boundary(text[..start].chars().next_back()) && boundary(text[end..].chars().next()) {
            return true;
        }
        from = start + 1;
    }
    false
}

/// One level of nesting removed, up to `depth` times.
fn flatten(items: &[Value], depth: usize) -> Vec<Value> {
    let mut out = Vec::new();
    for item in items {
        match item {
            Value::Array(inner) if depth > 0 => out.extend(flatten(inner, depth - 1)),
            other => out.push(other.clone()),
        }
    }
    out
}

/// Which way a pointer action goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    Click,
    Move,
    Down,
    Up,
}

/// One thing to do to a screen, after the model's vocabulary has been reduced
/// to the machine's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Pointer {
        x: i64,
        y: i64,
        press: Press,
        /// `true` for the right button.
        right: bool,
    },
    /// Typing. Named for how it is done rather than for what it means: the text
    /// is pasted, not typed. See [`action_command`].
    Clipboard {
        text: String,
    },
    Key {
        key: String,
        modifiers: Vec<String>,
    },
    Scroll {
        up: bool,
        amount: i64,
    },
    Wait {
        ms: i64,
    },
}

impl Action {
    /// Where a click landed, for the close-up's caption. `None` for anything
    /// that is not a press: a batch that only moved the pointer has nothing
    /// new under it to look at.
    fn pressed_at(&self) -> Option<(i64, i64)> {
        match self {
            Self::Pointer { x, y, press, .. } if *press != Press::Move => Some((*x, *y)),
            _ => None,
        }
    }
}

/// The last press in a batch, which is what a close-up is taken of.
pub fn last_press(actions: &[Action]) -> Option<(i64, i64)> {
    actions.iter().rev().find_map(Action::pressed_at)
}

/// The model's action list, checked and normalised.
///
/// The model writes in a vocabulary a person would recognise - click, type,
/// key, scroll, wait - and this turns it into the smaller set the machine
/// actually has. A double click becomes two clicks, because that is what a
/// double click is and expanding it here means the compiler below has one case
/// instead of two.
///
/// Fails rather than skipping. An action list where item four was silently
/// dropped executes items five and six against a screen that never got item
/// four, which is how an agent ends up typing a password into a page that never
/// opened.
pub fn parse_actions(value: Option<&Value>, extra: &Value) -> Result<Vec<Action>, String> {
    let lifted = lift_actions(value, extra)?;
    if lifted.is_empty() {
        return Err(format!("This needs at least one action, like {EXAMPLE}."));
    }
    if lifted.len() > MAX_ACTIONS {
        return Err(format!("At most {MAX_ACTIONS} actions in one batch."));
    }

    let mut actions: Vec<Action> = Vec::new();
    for raw in &lifted {
        let kind = raw
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase();

        match kind.as_str() {
            "click" | "move" | "down" | "up" => {
                let pointer = Action::Pointer {
                    x: coordinate(raw.get("x"), "x")?,
                    y: coordinate(raw.get("y"), "y")?,
                    press: match kind.as_str() {
                        "move" => Press::Move,
                        "down" => Press::Down,
                        "up" => Press::Up,
                        _ => Press::Click,
                    },
                    right: raw.get("button").and_then(Value::as_str) == Some("right"),
                };
                let double = raw.get("double") == Some(&Value::Bool(true)) && kind == "click";
                actions.push(pointer.clone());
                if double {
                    actions.push(pointer);
                }
            }
            "type" => {
                // `text` is right, but a model that has just written a key
                // action often reaches for `key` here too. There is no other
                // thing it could mean.
                let text = raw
                    .get("text")
                    .or_else(|| raw.get("key"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                actions.push(Action::Clipboard { text });
            }
            "key" => {
                // Accept `text` as well as `key`.
                //
                // Not sloppiness - measured. A real model, told the field is
                // `key`, sent `{kind: "key", text: "Return"}` five times in a
                // row against five different rejections, because `type` takes
                // `text` and the two blur together. It has exactly one meaning,
                // and a parser that refuses to read it is choosing to be right
                // over being useful.
                let mut modifiers: Vec<String> = raw
                    .get("modifiers")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .map(|item| match item {
                                Value::String(s) => s.clone(),
                                other => other.to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mut key = raw
                    .get("key")
                    .or_else(|| raw.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();

                // The whole combo in the modifier list, with no key at all.
                //
                // Also measured. `{kind: "key", modifiers: ["alt", "Tab"]}` is
                // what a model writes when it is thinking of the chord as one
                // thing, and it is not ambiguous: Tab is not a modifier, so it
                // is the key. Refusing it cost a user six consecutive failed
                // batches - the same rejection six times, because nothing in
                // the error told the model which of its two fields was wrong.
                if key.is_empty() {
                    let spare: Vec<&String> =
                        modifiers.iter().filter(|name| !is_modifier(name)).collect();
                    if spare.len() == 1 {
                        key = spare[0].clone();
                        modifiers.retain(|name| is_modifier(name));
                    }
                }

                if key.is_empty() {
                    return Err(format!(
                        r#"A key action needs a key: {{"kind":"key","key":"Return"}}. Modifiers go in their own list: {{"kind":"key","key":"a","modifiers":["ctrl"]}}. You sent {raw}."#
                    ));
                }
                actions.push(Action::Key { key, modifiers });
            }
            "scroll" => actions.push(Action::Scroll {
                up: raw.get("direction").and_then(Value::as_str) == Some("up"),
                amount: bounded(raw.get("amount"), 1, 20, 3),
            }),
            "wait" => actions.push(Action::Wait {
                ms: bounded(raw.get("ms"), 0, MAX_WAIT_MS, 350),
            }),
            other => {
                let named = if other.is_empty() { "(missing)" } else { other };
                return Err(format!(
                    "Unsupported action \"{named}\". The kinds are {}, and a call looks like {EXAMPLE}.",
                    KINDS.join(", ")
                ));
            }
        }
    }

    if actions.len() > MAX_ACTIONS {
        return Err(format!(
            "That expands to more than {MAX_ACTIONS} actions. Split the batch."
        ));
    }
    Ok(actions)
}

/// One action, as a shell line.
///
/// ## Why typing is a paste
///
/// Text is put on the clipboard and pasted, not typed. `xdotool type`
/// synthesises a key event per character, which means it is subject to the
/// keyboard layout, drops characters into any field that does something on
/// keystroke - an address bar with autocomplete, a search box that refetches -
/// and takes a second to write a URL. A paste is one event and arrives whole.
/// The fallback to `type` is there for a machine with no xclip, where slow and
/// lossy beats nothing.
pub fn action_command(action: &Action, humanize: bool, seed: u64) -> String {
    match action {
        // Buttons 4 and 5 are the wheel. There is no other way to say this in
        // X.
        Action::Scroll { up, amount } => format!(
            "xdotool click --repeat {amount} {}",
            if *up { 4 } else { 5 }
        ),
        Action::Key { key, modifiers } => {
            let mut parts = modifiers.clone();
            parts.push(key.clone());
            format!(
                "xdotool key --clearmodifiers {}",
                quote(&parts.join("+"))
            )
        }
        Action::Clipboard { text } => format!(
            "if command -v xclip >/dev/null 2>&1; then printf %s {quoted} | xclip -selection clipboard -i; \
             xdotool key --clearmodifiers ctrl+v; \
             else xdotool type --clearmodifiers --delay 12 -- {quoted}; fi",
            quoted = quote(text)
        ),
        Action::Wait { ms } => format!("sleep {:.3}", *ms as f64 / 1000.0),
        Action::Pointer { x, y, press, right } => {
            let button = if *right { 3 } else { 1 };
            // The walk is best effort; the exact move is not, and it is what
            // makes the click land on the pixel the caller named.
            let at = format!(
                "{}{}",
                if humanize { glide(*x, *y, seed) } else { String::new() },
                arrive(*x, *y)
            );
            match press {
                Press::Move => at,
                Press::Down => format!("{at} mousedown {button}"),
                Press::Up => format!("{at} mouseup {button}"),
                Press::Click if !humanize => format!("{at} click {button}"),
                Press::Click => {
                    // A rest on arrival and a held press, rather than xdotool's
                    // instant click. Settling before pressing is the other half
                    // of moving like a hand, and a button that measures how long
                    // it was held sees a plausible number.
                    let (rest, hold) = press_timing(seed);
                    format!(
                        "{at} sleep {:.3} mousedown {button} sleep {:.3} mouseup {button}",
                        rest as f64 / 1000.0,
                        hold as f64 / 1000.0
                    )
                }
            }
        }
    }
}

/* -- how a pointer gets where it is going ---------------------------------- */

/// `xdotool mousemove x y` teleports. Nothing on a real desktop moves that way,
/// and two things notice. The obvious one is the site: a pointer that appears
/// inside a button having never crossed the page is one of the cheapest bot
/// signals there is, and it is checked for by everything from a bank login to a
/// checkout. The quieter one is the page itself. A menu that opens on hover, a
/// dropdown that arms on mouseenter, a canvas that tracks movement - all of
/// them are driven by the events a real move produces and a jump does not, so a
/// click that lands on an item of a menu that never opened hits the page
/// behind it.
///
/// So the pointer is walked along a cubic Bezier from where it actually is to
/// where it is going: two control points pushed off the straight line by a
/// random amount and a random side, a velocity curve that accelerates hard and
/// decelerates long, a pixel or two of jitter on the way, and over a long
/// distance a small overshoot that the final move corrects.
///
/// The path is computed on the machine because the curve has to start from the
/// pointer's current position, and this process does not know it. Reading it
/// back first would be a round trip per move. `awk` is in every image this can
/// run on, and xdotool takes a whole chain of commands in one invocation, so
/// the entire path is one process.
///
/// Accuracy is not traded away: an exact `mousemove --sync` always follows
/// outside the awk. Every source of randomness is in the journey.
///
/// Below this many pixels there is no journey worth drawing.
pub const GLIDE_NEAR: i64 = 6;

/// The path, as an awk program printing xdotool's own chain syntax.
///
/// One line, because a `#` comment inside it would swallow the rest of the
/// program. No single quotes, because it is passed to the shell inside them.
pub const GLIDE_AWK: &str = concat!(
    "BEGIN{",
    "sx=sx+0;sy=sy+0;tx=tx+0;ty=ty+0;",
    "if(sx<0||sy<0)exit;",
    "dx=tx-sx;dy=ty-sy;d=sqrt(dx*dx+dy*dy);",
    "if(d<near)exit;",
    "srand(seed);",
    "n=int(d/14)+7;if(n>40)n=40;",
    "ux=-dy/d;uy=dx/d;",
    "over=0;if(d>240)over=3+rand()*5;",
    "ex=tx+dx/d*over;ey=ty+dy/d*over;",
    "dx=ex-sx;dy=ey-sy;",
    "a1=0.15+rand()*0.2;a2=0.55+rand()*0.2;",
    "bow=d*(0.06+rand()*0.12);if(bow>90)bow=90;",
    "side=1;if(rand()<0.5)side=-1;",
    "o1=bow*side;o2=bow*side*(0.35-rand()*1.1);",
    "c1x=sx+dx*a1+ux*o1;c1y=sy+dy*a1+uy*o1;",
    "c2x=sx+dx*a2+ux*o2;c2y=sy+dy*a2+uy*o2;",
    "jit=1+d/500;if(jit>2.5)jit=2.5;",
    "span=90+d*0.5;if(span>620)span=620;",
    "lx=-1;ly=-1;",
    "for(i=1;i<=n;i++){",
    "t=i/n;",
    "e=1-(1-t)^2.6;e=0.35*(t*t*(3-2*t))+0.65*e;m=1-e;",
    "bx=m*m*m*sx+3*m*m*e*c1x+3*m*e*e*c2x+e*e*e*ex;",
    "by=m*m*m*sy+3*m*m*e*c1y+3*m*e*e*c2y+e*e*e*ey;",
    "if(i<n){bx+=(rand()-0.5)*2*jit;by+=(rand()-0.5)*2*jit;}",
    "px=int(bx+0.5);py=int(by+0.5);",
    "if(px<0)px=0;if(py<0)py=0;",
    "if(px==lx&&py==ly)continue;",
    "lx=px;ly=py;",
    "dt=(span/n)*(0.55+0.5*t+rand()*0.5)/1000;if(dt<0.004)dt=0.004;",
    "printf \"mousemove %d %d sleep %.3f \",px,py,dt;",
    "}",
    "}",
);

/// A different shape every time, and a number a test can pin.
///
/// The clock rather than a random number generator, so the crate needs no
/// dependency for something whose only requirement is "not the same twice".
/// Nothing here is security-sensitive: a predictable path is still a path.
pub fn glide_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.subsec_nanos() as u64 ^ (since.as_secs() << 12))
        .unwrap_or(1)
        % 1_000_000_000
}

/// The walk to (x, y), as shell. Empty on a machine without awk, and harmless
/// on one where the pointer cannot be located: both fall through to the exact
/// move that always follows.
///
/// `$GP` is deliberately unquoted. It is a list of xdotool arguments generated
/// by the program above - numbers, and the words `mousemove` and `sleep` - and
/// splitting it into words is the entire point.
pub fn glide(x: i64, y: i64, seed: u64) -> String {
    format!(
        "if command -v awk >/dev/null 2>&1; then \
         GX=-1; GY=-1; \
         eval \"$(xdotool getmouselocation --shell 2>/dev/null | sed -n -e s/^X=/GX=/p -e s/^Y=/GY=/p)\"; \
         GP=$(awk -v sx=\"$GX\" -v sy=\"$GY\" -v tx={x} -v ty={y} -v near={GLIDE_NEAR} -v seed={seed} \
         {program} 2>/dev/null); \
         [ -n \"$GP\" ] && xdotool $GP; \
         fi; ",
        program = quote(GLIDE_AWK)
    )
}

/// The move that lands on the exact pixel - and why it is two moves.
///
/// `xdotool mousemove --sync` waits for a motion event confirming the pointer
/// arrived. If the pointer is ALREADY on that pixel there is no motion to wait
/// for, and it does not notice: it blocks for fifteen seconds and then gives
/// up. Measured in the sandbox image - 2ms when the pointer has to move, 15,114
/// when it does not.
///
/// That was always reachable: a double click is two clicks at one point, and so
/// is any batch that clicks the same button twice. The curve above made it the
/// common case rather than the rare one, because a short path ends exactly on
/// the target, so every close-together click paid the full fifteen seconds.
///
/// So a pixel of movement is guaranteed: step one row away without waiting -
/// which is free, since only `--sync` can block - and then sync onto the target
/// from somewhere that is definitely not it. Up rather than down, so a click on
/// the last row of the screen does not aim off the edge.
pub fn arrive(x: i64, y: i64) -> String {
    let from = if y > 0 { y - 1 } else { y + 1 };
    format!("xdotool mousemove {x} {from} mousemove --sync {x} {y}")
}

/// How long a press is held, and how long the pointer rests before it.
///
/// Derived from the same seed as the path rather than drawn again, so one seed
/// shapes the whole gesture and a test that pins it pins the command.
fn press_timing(seed: u64) -> (u64, u64) {
    (30 + seed % 40, 50 + (seed >> 7) % 70)
}

/* -- looking --------------------------------------------------------------- */

/// How far apart the ruler marks are, in pixels.
const RULER_STEP: i64 = 100;

/// The most a ruler is drawn out to; a smaller screen simply loses the rest.
const RULER_MAX: i64 = 4000;

/// The close-up around a click: this much screen, shown at twice the size.
const CLOSEUP_WIDTH: i64 = 400;
const CLOSEUP_HEIGHT: i64 = 240;
const CLOSEUP_SCALE: i64 = 2;

/// Rulers along the top and left edges, and a ring where the pointer is.
///
/// Written because of what a model does without them. Shown a plain 1600x900
/// screenshot, a model read a button that sat at y=662 as being at y=735 and a
/// field at y=610 as being at y=658 - seventy pixels out, which is the next
/// control down - and then, told "cursor at 1052,735" in words, could not
/// relate the number to anything in the picture. It clicked the terms text
/// under the button four times. Numbered marks every hundred pixels give it
/// something to read a coordinate off, and the ring shows it exactly where its
/// last click landed against the thing it meant to hit.
///
/// Drawn with ImageMagick on the machine, since it is already there for the
/// screenshot, and skipped without complaint on a machine that lacks it.
fn annotation() -> String {
    let mut draws: Vec<String> = Vec::new();
    let mut x = RULER_STEP;
    while x <= RULER_MAX {
        draws.push(format!(
            "-fill 'rgba(0,0,0,0.6)' -stroke none -draw 'rectangle {},0 {},11'",
            x - 15,
            x + 15
        ));
        draws.push(format!(
            "-fill white -pointsize 10 -annotate +{}+9 '{x}'",
            x - 12
        ));
        draws.push(format!(
            "-fill none -stroke 'rgba(255,255,255,0.5)' -strokewidth 1 -draw 'line {x},12 {x},18'"
        ));
        x += RULER_STEP;
    }
    let mut y = RULER_STEP;
    while y <= RULER_MAX {
        draws.push(format!(
            "-fill 'rgba(0,0,0,0.6)' -stroke none -draw 'rectangle 0,{} 28,{}'",
            y - 6,
            y + 6
        ));
        draws.push(format!(
            "-fill white -pointsize 10 -annotate +2+{} '{y}'",
            y + 4
        ));
        draws.push(format!(
            "-fill none -stroke 'rgba(255,255,255,0.5)' -strokewidth 1 -draw 'line 29,{y} 35,{y}'"
        ));
        y += RULER_STEP;
    }
    // The pointer: a red ring at the cursor, from the coordinates read a moment
    // ago. `$X`/`$Y` are set by `eval` of xdotool's --shell output.
    draws.push(
        r#"-fill none -stroke red -strokewidth 2 -draw "circle $X,$Y $((X+8)),$Y""#.to_string(),
    );
    draws.push(r#"-fill red -stroke none -draw "circle $X,$Y $((X+2)),$Y""#.to_string());

    format!(
        r#"if command -v convert >/dev/null 2>&1 && [ -n "$X" ]; then convert - {} png:-; else cat; fi"#,
        draws.join(" ")
    )
}

/// A screen, and what is on it.
///
/// One command rather than five, because five is five exec round trips and over
/// a cloud provider each is an HTTP call to another continent. The metadata
/// goes first as `KEY=value` lines and the image last as one base64 blob, so a
/// partial read still yields the geometry.
///
/// The title is base64'd on its way out. A window called `Bill "Bob" O'Hara -
/// Chromium` otherwise ends the line early and takes the parser with it.
///
/// `annotate` draws the rulers and the pointer ring for a model's eyes; the
/// window's own screenshot path never asks for them.
fn observe_command(annotate: bool) -> String {
    [
        WITH_DISPLAY.to_string(),
        NEEDS_DISPLAY.to_string(),
        r#"set -- $(xdotool getdisplaygeometry 2>/dev/null || printf '1600 900');"#.into(),
        r#"printf 'WIDTH=%s\nHEIGHT=%s\n' "$1" "$2";"#.into(),
        r#"X=; Y=; eval "$(xdotool getmouselocation --shell 2>/dev/null | grep -E '^(X|Y)=' || true)";"#.into(),
        r#"[ -n "$X" ] && printf 'X=%s\nY=%s\n' "$X" "$Y";"#.into(),
        r#"window=$(xdotool getactivewindow 2>/dev/null || true);"#.into(),
        r#"printf 'WINDOW=%s\n' "$window";"#.into(),
        r#"if [ -n "$window" ]; then "#.into(),
        r#"  printf 'TITLE=%s\n' "$(xdotool getwindowname "$window" 2>/dev/null | base64 -w0 || true)"; "#.into(),
        r#"fi;"#.into(),
        // compression-level 3 rather than the default 7. The picture crosses an
        // exec boundary as base64 and is looked at once; spending 200ms to save
        // 40KB is the wrong trade every time.
        r#"printf 'IMAGE=';"#.into(),
        format!(
            "import -window root -define png:compression-level=3 png:- 2>/dev/null{} | base64 -w 0;",
            if annotate {
                format!(" | {}", annotation())
            } else {
                String::new()
            }
        ),
        r#"printf '\n'"#.into(),
    ]
    .join(" ")
}

/// A screen, with no rulers. For the window's own still frames.
pub fn observe() -> String {
    observe_command(false)
}

/// Just the picture, base64, for the still frames the window draws.
///
/// ImageMagick's `import`, which is what the sandbox image actually carries -
/// both providers used to ask for `scrot`, which is not installed, so
/// `command -v scrot` failed, the answer was empty, and every machine reported
/// "No display on this machine" while its desktop was running perfectly. The
/// two fallbacks are for an image somebody built themselves.
pub fn still() -> String {
    [
        "if command -v import >/dev/null 2>&1; then",
        "  import -window root -define png:compression-level=3 png:- 2>/dev/null | base64 -w0;",
        "elif command -v scrot >/dev/null 2>&1; then",
        "  scrot -o /tmp/inertia-screen.png && base64 -w0 /tmp/inertia-screen.png;",
        "elif command -v xwd >/dev/null 2>&1; then",
        "  xwd -root -silent | base64 -w0;",
        "fi",
    ]
    .join(" ")
}

/// The same, with the rulers and the pointer ring, for the model.
pub fn observe_annotated() -> String {
    observe_command(true)
}

/// A magnified look at where a click went.
///
/// The screenshot says where the pointer is; this says what is under it. Four
/// hundred by two hundred and forty pixels around the point, shown at twice the
/// size with a crosshair on the exact pixel, is enough to tell a text field
/// from the label above it and a button from the paragraph below - which is the
/// difference between a model that corrects a miss and one that types a
/// password into a terms-of-service notice.
pub fn closeup_command(x: i64, y: i64) -> String {
    let left = (x - CLOSEUP_WIDTH / 2).max(0);
    let top = (y - CLOSEUP_HEIGHT / 2).max(0);
    let cx = (x - left) * CLOSEUP_SCALE;
    let cy = (y - top) * CLOSEUP_SCALE;
    let w = CLOSEUP_WIDTH * CLOSEUP_SCALE;
    let h = CLOSEUP_HEIGHT * CLOSEUP_SCALE;
    format!(
        "if command -v convert >/dev/null 2>&1; then printf 'CLOSEUP='; \
         import -window root -define png:compression-level=3 png:- 2>/dev/null | \
         convert - -crop {CLOSEUP_WIDTH}x{CLOSEUP_HEIGHT}+{left}+{top} +repage -resize {}% \
         -fill none -stroke red -strokewidth 2 \
         -draw 'line {cx},0 {cx},{h}' -draw 'line 0,{cy} {w},{cy}' \
         -draw 'circle {cx},{cy} {},{cy}' png:- | base64 -w 0; printf '\\n'; fi;",
        CLOSEUP_SCALE * 100,
        cx + 12
    )
}

/// A batch: do these, let the screen settle, then look.
///
/// `settle_ms` is not politeness. A click that opens a menu returns before the
/// menu is drawn, so an observation taken immediately shows the screen as it
/// was and the model clicks the next thing against a picture of the last one.
pub fn batch(actions: &[Action], settle_ms: Option<i64>, observe_after: bool) -> String {
    batch_with_seed(actions, settle_ms, observe_after, true, glide_seed())
}

/// The same, with the randomness pinned. The seam a test drives.
pub fn batch_with_seed(
    actions: &[Action],
    settle_ms: Option<i64>,
    observe_after: bool,
    humanize: bool,
    seed: u64,
) -> String {
    let mut lines = vec![WITH_DISPLAY.to_string(), NEEDS_DISPLAY.to_string()];
    for action in actions {
        lines.push(format!("{};", action_command(action, humanize, seed)));
    }
    let wait = bounded(settle_ms.map(Value::from).as_ref(), 0, MAX_SETTLE_MS, 350);
    if wait > 0 {
        lines.push(format!("sleep {:.3};", wait as f64 / 1000.0));
    }
    lines.push(format!("printf 'COMPLETED=%s\\n' {};", actions.len()));
    if observe_after {
        // The close-up goes with the picture, and only after a click - a batch
        // that only typed has nothing new under the pointer to look at.
        if let Some((x, y)) = last_press(actions) {
            lines.push(closeup_command(x, y));
        }
        lines.push(observe_annotated());
    }
    lines.join(" ")
}

/// Open a file or a URL in whatever handles it.
///
/// `xdg-open` rather than naming the browser, because the machine already knows
/// what opens a PDF and this way a spreadsheet does not open in Chromium. The
/// browser is registered as the http handler at boot, so a URL still lands
/// where it should.
///
/// Backgrounded with nohup: xdg-open blocks for as long as the application runs
/// on some handlers, and an agent that opens a document should not hang until
/// someone closes it.
pub fn open(target: &str) -> String {
    [
        WITH_DISPLAY.to_string(),
        NEEDS_DISPLAY.to_string(),
        r#"if ! command -v xdg-open >/dev/null 2>&1; then echo "This machine cannot open files: no xdg-open."; exit 3; fi;"#.into(),
        format!("nohup xdg-open {} >/tmp/inertia/open.log 2>&1 &", quote(target)),
        "sleep 2;".into(),
        r#"echo "opened";"#.into(),
    ]
    .join(" ")
}

/// Start an application by name.
///
/// "browser" is special-cased to a list because it is the thing asked for most
/// and the name differs by image. Anything else is taken literally, checked
/// with `command -v` first so a typo is "there is no such application" rather
/// than a shell error.
pub fn launch(application: &str, uri: Option<&str>) -> String {
    let names: Vec<String> = if application.to_lowercase() == "browser" {
        ["inertia-browser", "chromium", "chromium-browser", "firefox"]
            .iter()
            .map(|name| (*name).to_string())
            .collect()
    } else {
        vec![application.to_string()]
    };
    let safe: String = application
        .chars()
        .filter(|c| !matches!(c, '"' | '\'' | '`' | '$'))
        .collect();

    [
        WITH_DISPLAY.to_string(),
        NEEDS_DISPLAY.to_string(),
        format!(
            "for app in {}; do",
            names
                .iter()
                .map(|name| quote(name))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        r#"  if command -v "$app" >/dev/null 2>&1; then"#.into(),
        format!(
            r#"    nohup "$app"{} >/tmp/inertia/app.log 2>&1 &"#,
            uri.map(|u| format!(" {}", quote(u))).unwrap_or_default()
        ),
        r#"    sleep 2; echo "launched $app"; exit 0;"#.into(),
        "  fi;".into(),
        "done;".into(),
        format!(r#"echo "No application called {safe} on this machine."; exit 3"#),
    ]
    .join(" ")
}

/// The text of the page the browser is showing.
///
/// Select all, copy, read the clipboard - because there is no debugging port on
/// this browser and adding one would mean a protocol client. Crude, and it is
/// the difference between an agent that can read a page and one that can only
/// look at a picture of it. It is also the only thing that works when the model
/// has no eyes.
pub fn page_text() -> String {
    [
        WITH_DISPLAY,
        NEEDS_DISPLAY,
        r#"if ! command -v xclip >/dev/null 2>&1; then echo "This machine has no clipboard tool."; exit 3; fi;"#,
        r#"xdotool search --onlyvisible --class '[Cc]hromium' windowactivate --sync key --clearmodifiers ctrl+a;"#,
        "sleep 0.3;",
        "xdotool key --clearmodifiers ctrl+c;",
        "sleep 0.5;",
        "xclip -selection clipboard -o 2>/dev/null | head -c 60000;",
        // Leave nothing selected, or the next screenshot is a page of blue.
        "xdotool key --clearmodifiers ctrl+shift+Home >/dev/null 2>&1 || true",
    ]
    .join(" ")
}

/// Everything on screen, as a list a model can act on.
pub fn windows() -> String {
    [
        WITH_DISPLAY,
        NEEDS_DISPLAY,
        r#"xdotool search --onlyvisible --name "." 2>/dev/null | while read -r id; do"#,
        r#"  name=$(xdotool getwindowname "$id" 2>/dev/null);"#,
        r#"  geo=$(xdotool getwindowgeometry --shell "$id" 2>/dev/null | tr '\n' ' ');"#,
        r#"  [ -n "$name" ] && echo "$id | $name | $geo";"#,
        "done",
    ]
    .join(" ")
}

pub fn close_browser() -> String {
    "pgrep -x chromium >/dev/null 2>&1 && pkill -x chromium; echo closed".to_string()
}

/// What the machine said it saw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Observation {
    pub width: i64,
    pub height: i64,
    pub cursor: Option<(i64, i64)>,
    pub window_id: Option<String>,
    pub window_title: Option<String>,
    /// Base64 PNG, or `None` when the screen could not be photographed.
    pub image: Option<String>,
    pub closeup: Option<String>,
    pub completed: usize,
}

/// Read one observation out of what the command printed.
///
/// Tolerant on purpose. A machine that answered with geometry and no image is
/// still telling us something true, and the caller can say "the screen is
/// 1600x900 and I could not photograph it" - which is a better error than a
/// parse failure.
pub fn parse_observation(stdout: &str) -> Observation {
    let field = |name: &str| -> Option<&str> {
        let needle = format!("{name}=");
        stdout.lines().find_map(|line| {
            // Anchored at the start of a line, the way the JS regex is: a
            // base64 blob can contain anything, and a loose search would find
            // "X=" inside the picture.
            line.strip_prefix(needle.as_str())
        })
    };
    let positive = |name: &str, fallback: i64| -> i64 {
        field(name)
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .map_or(fallback, |value| value as i64)
    };

    let png = |name: &str| -> Option<String> {
        field(name)
            .map(str::trim)
            // The magic number, rather than a size threshold. A screen painted
            // one flat colour compresses to about 350 bytes, so "bigger than
            // 512 bytes" reports an idle machine as having no display at all.
            .filter(|text| text.starts_with("iVBORw0K"))
            .map(str::to_string)
    };

    let x = field("X").and_then(|v| v.trim().parse::<i64>().ok());
    let y = field("Y").and_then(|v| v.trim().parse::<i64>().ok());

    let window_title = field("TITLE")
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .and_then(|encoded| {
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()
        })
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty());

    Observation {
        width: positive("WIDTH", 1600),
        height: positive("HEIGHT", 900),
        cursor: x.zip(y),
        window_id: field("WINDOW")
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string),
        window_title,
        image: png("IMAGE"),
        closeup: png("CLOSEUP"),
        completed: field("COMPLETED")
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0),
    }
}

/// A short name for a frame, so an unchanged screen can be recognised.
///
/// The whole point is not sending the same picture twice. A model looking at a
/// conversation that carries six identical screenshots of a loading page has
/// spent most of its context on the fact that nothing happened; told "screen
/// unchanged" it gets on with waiting.
///
/// FNV-1a over the base64, exactly as the other shell did, so a frame name is
/// the same string in both and a test written against one holds for the other.
pub fn frame_id(image: Option<&str>) -> Option<String> {
    let image = image?;
    let mut hash: u32 = 0x811c_9dc5;
    for byte in image.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    Some(format!(
        "{}-{}",
        base36(image.len() as u64),
        base36(u64::from(hash))
    ))
}

/// Base 36, the way `Number.prototype.toString(36)` writes it.
fn base36(mut value: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    /// The one that cost every machine its preview.
    ///
    /// Both providers asked for `scrot`, which the sandbox image does not
    /// install - it carries ImageMagick, and the observe path has always used
    /// `import`. `command -v scrot` failed, the answer came back empty, and the
    /// chat sidebar said "No display on this machine" about a machine whose
    /// desktop was running.
    #[test]
    fn a_still_is_taken_with_a_tool_the_image_actually_has() {
        let command = still();
        assert!(command.contains("import -window root"), "{command}");
        assert!(
            command.contains("base64 -w0"),
            "it has to cross an exec boundary"
        );
        // The fallbacks are for an image somebody built themselves, and they
        // come after, not instead.
        assert!(command.find("import").unwrap() < command.find("scrot").unwrap());
    }

    use super::*;
    use serde_json::json;

    const NONE: &Value = &Value::Null;

    #[test]
    fn a_plain_list_of_actions_parses() {
        let actions = parse_actions(
            Some(&json!([
                {"kind": "click", "x": 640, "y": 360},
                {"kind": "type", "text": "hello"},
                {"kind": "key", "key": "Return"}
            ])),
            NONE,
        )
        .unwrap();
        assert_eq!(actions.len(), 3);
        assert_eq!(
            actions[0],
            Action::Pointer {
                x: 640,
                y: 360,
                press: Press::Click,
                right: false
            }
        );
    }

    /// Measured: a model sends the fields of one action at the top level with
    /// no list at all. Refusing it cost a whole turn.
    #[test]
    fn one_action_at_the_top_level_is_read_as_a_list_of_one() {
        let call = json!({"kind": "click", "x": 10, "y": 20});
        let actions = parse_actions(None, &call).unwrap();
        assert_eq!(
            actions,
            [Action::Pointer {
                x: 10,
                y: 20,
                press: Press::Click,
                right: false
            }]
        );
    }

    /// `{"actions":["click"],"x":10,"y":20}` is a click at 10,20.
    #[test]
    fn a_bare_kind_takes_its_coordinates_from_beside_the_list() {
        let call = json!({"actions": ["click"], "x": 10, "y": 20});
        let actions = parse_actions(call.get("actions"), &call).unwrap();
        assert_eq!(
            actions,
            [Action::Pointer {
                x: 10,
                y: 20,
                press: Press::Click,
                right: false
            }]
        );
    }

    #[test]
    fn a_kind_buried_in_a_string_is_still_a_kind() {
        let call = json!({"actions": ["kind>click"], "x": 5, "y": 5});
        let actions = parse_actions(call.get("actions"), &call).unwrap();
        assert!(matches!(actions[0], Action::Pointer { x: 5, .. }));
    }

    /// A button name tacked on after the action it belongs to.
    #[test]
    fn a_trailing_button_name_joins_the_click_before_it() {
        let call = json!({"actions": [{"kind": "click", "x": 507, "y": 527}, "right"]});
        let actions = parse_actions(call.get("actions"), &call).unwrap();
        assert_eq!(
            actions,
            [Action::Pointer {
                x: 507,
                y: 527,
                press: Press::Click,
                right: true
            }]
        );
    }

    /// A list inside the list is one action wrapped twice.
    #[test]
    fn a_doubly_wrapped_action_is_unwrapped() {
        let call = json!({"actions": [[{"kind": "wait", "ms": 500}]]});
        let actions = parse_actions(call.get("actions"), &call).unwrap();
        assert_eq!(actions, [Action::Wait { ms: 500 }]);
    }

    /// Measured five times in a row against five rejections.
    #[test]
    fn a_key_action_written_with_text_is_read() {
        let call = json!({"actions": [{"kind": "key", "text": "Return"}]});
        let actions = parse_actions(call.get("actions"), &call).unwrap();
        assert_eq!(
            actions,
            [Action::Key {
                key: "Return".into(),
                modifiers: Vec::new()
            }]
        );
    }

    /// `{"kind":"key","modifiers":["alt","Tab"]}`: Tab is not a modifier, so it
    /// is the key.
    #[test]
    fn a_chord_written_entirely_as_modifiers_finds_its_key() {
        let call = json!({"actions": [{"kind": "key", "modifiers": ["alt", "Tab"]}]});
        let actions = parse_actions(call.get("actions"), &call).unwrap();
        assert_eq!(
            actions,
            [Action::Key {
                key: "Tab".into(),
                modifiers: vec!["alt".into()]
            }]
        );
    }

    #[test]
    fn a_key_action_with_no_key_at_all_says_which_field_was_wrong() {
        let call = json!({"actions": [{"kind": "key", "modifiers": ["ctrl", "shift"]}]});
        let message = parse_actions(call.get("actions"), &call).unwrap_err();
        assert!(message.contains(r#""kind":"key","key":"Return""#));
    }

    #[test]
    fn a_double_click_becomes_two_clicks() {
        let call = json!({"actions": [{"kind": "click", "x": 1, "y": 1, "double": true}]});
        let actions = parse_actions(call.get("actions"), &call).unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0], actions[1]);
    }

    #[test]
    fn an_empty_batch_is_refused_with_the_example() {
        let message = parse_actions(Some(&json!([])), NONE).unwrap_err();
        assert!(message.contains(EXAMPLE));
    }

    #[test]
    fn an_unknown_kind_lists_the_ones_that_exist() {
        let call = json!({"actions": [{"kind": "teleport"}]});
        let message = parse_actions(call.get("actions"), &call).unwrap_err();
        assert!(message.contains("teleport"));
        assert!(message.contains("scroll"));
    }

    #[test]
    fn a_batch_longer_than_the_ceiling_is_refused() {
        let many: Vec<Value> = (0..MAX_ACTIONS + 1)
            .map(|_| json!({"kind": "wait", "ms": 1}))
            .collect();
        let message = parse_actions(Some(&json!(many)), NONE).unwrap_err();
        assert!(message.contains("At most"));
    }

    /// A double click at the ceiling expands past it, and that is a different
    /// sentence: the model has to split rather than shorten by one.
    #[test]
    fn an_expansion_past_the_ceiling_says_to_split() {
        let many: Vec<Value> = (0..MAX_ACTIONS)
            .map(|_| json!({"kind": "click", "x": 1, "y": 1, "double": true}))
            .collect();
        let message = parse_actions(Some(&json!(many)), NONE).unwrap_err();
        assert!(message.contains("Split the batch"));
    }

    #[test]
    fn a_wait_is_clamped_to_its_own_much_longer_ceiling() {
        let call = json!({"actions": [{"kind": "wait", "ms": 900_000}]});
        let actions = parse_actions(call.get("actions"), &call).unwrap();
        assert_eq!(actions, [Action::Wait { ms: MAX_WAIT_MS }]);
    }

    #[test]
    fn a_negative_coordinate_is_refused_rather_than_clamped() {
        let call = json!({"actions": [{"kind": "click", "x": -4, "y": 10}]});
        assert!(parse_actions(call.get("actions"), &call).is_err());
    }

    /* -- the commands ----------------------------------------------------- */

    #[test]
    fn typing_goes_through_the_clipboard_with_a_fallback() {
        let command = action_command(
            &Action::Clipboard {
                text: "pass'word".into(),
            },
            true,
            7,
        );
        assert!(command.contains("xclip -selection clipboard -i"));
        assert!(command.contains("ctrl+v"));
        assert!(command.contains("xdotool type"));
        // The quote in the text must not end the shell string.
        assert!(command.contains(r"'pass'\''word'"));
    }

    /// The exact move that follows the curve is what makes a click land on the
    /// pixel the caller named, and it never syncs onto where the pointer
    /// already is.
    #[test]
    fn arriving_always_steps_off_the_target_first() {
        assert_eq!(
            arrive(100, 200),
            "xdotool mousemove 100 199 mousemove --sync 100 200"
        );
        // The top row aims down rather than off the screen.
        assert_eq!(arrive(5, 0), "xdotool mousemove 5 1 mousemove --sync 5 0");
    }

    #[test]
    fn a_click_without_humanising_is_still_exact() {
        let command = action_command(
            &Action::Pointer {
                x: 10,
                y: 10,
                press: Press::Click,
                right: false,
            },
            false,
            1,
        );
        assert_eq!(
            command,
            "xdotool mousemove 10 9 mousemove --sync 10 10 click 1"
        );
    }

    #[test]
    fn a_humanised_click_walks_there_and_holds_the_button() {
        let command = action_command(
            &Action::Pointer {
                x: 800,
                y: 400,
                press: Press::Click,
                right: true,
            },
            true,
            12_345,
        );
        assert!(command.contains("getmouselocation"));
        assert!(command.contains("mousedown 3"));
        assert!(command.contains("mouseup 3"));
        // Pinned seed, pinned gesture: a test that pins the seed pins the
        // command.
        let again = action_command(
            &Action::Pointer {
                x: 800,
                y: 400,
                press: Press::Click,
                right: true,
            },
            true,
            12_345,
        );
        assert_eq!(command, again);
    }

    #[test]
    fn every_command_refuses_a_machine_with_no_desktop() {
        for command in [
            batch(&[Action::Wait { ms: 1 }], None, false),
            open("https://example.com"),
            launch("browser", None),
            page_text(),
            windows(),
            observe_annotated(),
        ] {
            assert!(command.contains("exit 3"), "no refusal in: {command}");
        }
    }

    #[test]
    fn a_batch_settles_before_it_looks_and_closes_up_on_the_last_click() {
        let actions = parse_actions(
            Some(&json!([
                {"kind": "click", "x": 100, "y": 100},
                {"kind": "type", "text": "hi"}
            ])),
            NONE,
        )
        .unwrap();
        let command = batch_with_seed(&actions, Some(1200), true, false, 3);
        assert!(command.contains("sleep 1.200;"));
        assert!(command.contains("COMPLETED=%s"));
        assert!(command.contains("CLOSEUP="));
        assert!(command.contains("IMAGE="));
    }

    #[test]
    fn a_batch_that_only_typed_takes_no_closeup() {
        let actions = parse_actions(Some(&json!([{"kind": "type", "text": "hi"}])), NONE).unwrap();
        let command = batch_with_seed(&actions, None, true, false, 3);
        assert!(!command.contains("CLOSEUP="));
    }

    #[test]
    fn not_asking_for_the_screen_leaves_the_picture_out() {
        let actions = parse_actions(Some(&json!([{"kind": "wait", "ms": 10}])), NONE).unwrap();
        let command = batch_with_seed(&actions, None, false, false, 3);
        assert!(!command.contains("IMAGE="));
    }

    #[test]
    fn opening_quotes_what_it_is_given() {
        let command = open("https://example.com/?a=1&b='2'");
        assert!(command.contains("xdg-open 'https://example.com/?a=1&b='\\''2'\\'''"));
    }

    #[test]
    fn launching_the_browser_tries_the_names_an_image_might_use() {
        let command = launch("browser", Some("https://example.com"));
        assert!(command.contains("'inertia-browser' 'chromium' 'chromium-browser' 'firefox'"));
        assert!(command.contains("'https://example.com'"));
    }

    #[test]
    fn launching_something_that_is_not_there_says_so_rather_than_failing_in_shell() {
        let command = launch("gimp\"; rm -rf /", None);
        assert!(command.contains("exit 3"));
        // The name reaches the loop inside single quotes, where a `"` or a `;`
        // is just a character.
        assert!(command.contains(r#"for app in 'gimp"; rm -rf /'; do"#));
        // And it is echoed back with the characters that would end the echo's
        // own double-quoted string taken out, so the sentence stays a sentence.
        assert!(command.contains(r#"echo "No application called gimp; rm -rf / on this machine.""#));
    }

    /* -- reading the answer ----------------------------------------------- */

    #[test]
    fn an_observation_is_read_out_of_the_key_value_lines() {
        let title = base64::engine::general_purpose::STANDARD.encode(r#"Bill "Bob" O'Hara"#);
        let stdout = format!(
            "WIDTH=1600\nHEIGHT=900\nX=120\nY=240\nWINDOW=12345\nTITLE={title}\nCOMPLETED=3\nIMAGE=iVBORw0Kabc\n"
        );
        let seen = parse_observation(&stdout);
        assert_eq!(seen.width, 1600);
        assert_eq!(seen.height, 900);
        assert_eq!(seen.cursor, Some((120, 240)));
        assert_eq!(seen.window_id.as_deref(), Some("12345"));
        assert_eq!(seen.window_title.as_deref(), Some(r#"Bill "Bob" O'Hara"#));
        assert_eq!(seen.image.as_deref(), Some("iVBORw0Kabc"));
        assert_eq!(seen.completed, 3);
    }

    /// A machine that answered with geometry and no picture is still telling us
    /// something true.
    #[test]
    fn geometry_without_a_picture_is_still_an_observation() {
        let seen = parse_observation("WIDTH=800\nHEIGHT=600\nIMAGE=\n");
        assert_eq!((seen.width, seen.height), (800, 600));
        assert_eq!(seen.image, None);
    }

    #[test]
    fn nothing_at_all_falls_back_rather_than_failing() {
        let seen = parse_observation("");
        assert_eq!((seen.width, seen.height), (1600, 900));
        assert_eq!(seen.cursor, None);
    }

    /// Anything that is not a PNG is not a picture. A flat colour compresses to
    /// about 350 bytes, so a size threshold would call an idle machine blind.
    #[test]
    fn something_that_is_not_a_png_is_not_treated_as_one() {
        let seen = parse_observation("IMAGE=bm90IGEgcG5n\n");
        assert_eq!(seen.image, None);
    }

    #[test]
    fn a_frame_name_is_stable_and_differs_per_picture() {
        let one = frame_id(Some("iVBORw0Kabc")).unwrap();
        assert_eq!(frame_id(Some("iVBORw0Kabc")).unwrap(), one);
        assert_ne!(frame_id(Some("iVBORw0Kabd")).unwrap(), one);
        assert_eq!(frame_id(None), None);
    }

    /// Pinned against the other shell's `frameId`, which produced this string
    /// for this input. The two have to agree or a frame a conversation has
    /// already seen reads as new.
    #[test]
    fn a_frame_name_matches_the_shell_this_was_ported_from() {
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
        // "a" is 0x61: 0x811c9dc5 ^ 0x61 = 0x811c9da4, times 0x01000193.
        let expected = {
            let hash = (0x811c_9dc5u32 ^ 0x61).wrapping_mul(0x0100_0193);
            format!("{}-{}", base36(1), base36(u64::from(hash)))
        };
        assert_eq!(frame_id(Some("a")).unwrap(), expected);
    }
}
