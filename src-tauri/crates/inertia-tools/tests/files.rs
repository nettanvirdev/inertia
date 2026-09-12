//! The file tools, exercised through the real registry against a real
//! temporary directory.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

use std::path::PathBuf;
use std::sync::Arc;

use inertia_core::message::ToolCall;
use inertia_core::tool::{ToolContext, ToolRegistry, ToolResult};
use inertia_core::{SessionId, ToolCallId};
use inertia_mock::MockGate;
use inertia_tools::{file_tools, ReadState, Registry};
use serde_json::json;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    registry: Registry,
    session: SessionId,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let gate = Arc::new(MockGate::allow_all());
        let registry = Registry::new(gate).with_tools(file_tools(Arc::new(ReadState::new()), Arc::new(inertia_lsp::Lsp::with_launcher(inertia_lsp::testing::fake_launcher()))));

        Self {
            _dir: dir,
            root,
            registry,
            session: SessionId::new(),
        }
    }

    fn write_file(&self, name: &str, contents: &str) {
        let path = self.root.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn read_file(&self, name: &str) -> String {
        std::fs::read_to_string(self.root.join(name)).unwrap()
    }

    async fn call(&self, tool: &str, args: serde_json::Value) -> ToolResult {
        let ctx = ToolContext {
            root: self.root.clone(),
            session: self.session.clone(),
            call_id: ToolCallId::new(),
            permissions: Arc::new(MockGate::allow_all()),
        };
        self.registry
            .run(
                &ToolCall {
                    id: ToolCallId::new(),
                    name: tool.to_string(),
                    arguments: args.to_string(),
                },
                &ctx,
            )
            .await
            .unwrap()
    }
}

// ── read ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn reading_numbers_the_lines() {
    let fixture = Fixture::new();
    fixture.write_file("a.txt", "first\nsecond\nthird\n");

    let result = fixture.call("read", json!({ "filePath": "a.txt" })).await;

    assert!(result.ok, "{}", result.output);
    assert!(result.output.contains("1: first"));
    assert!(result.output.contains("3: third"));
}

#[tokio::test]
async fn an_empty_file_says_so() {
    let fixture = Fixture::new();
    fixture.write_file("empty.txt", "");

    let result = fixture.call("read", json!({ "filePath": "empty.txt" })).await;
    assert!(result.ok);
    assert_eq!(result.output, "(Empty file)");
}

#[tokio::test]
async fn reading_can_be_paged() {
    let fixture = Fixture::new();
    let body: String = (1..=100).map(|i| format!("line {i}\n")).collect();
    fixture.write_file("long.txt", &body);

    let result = fixture
        .call("read", json!({ "filePath": "long.txt", "offset": 50, "limit": 3 }))
        .await;

    assert!(result.output.contains("50: line 50"));
    assert!(result.output.contains("52: line 52"));
    assert!(!result.output.contains("53: line 53"));
    // The model is told there is more, so it can page rather than assume it
    // has seen the file.
    assert!(result.output.contains("of 100"), "got {}", result.output);
}

#[tokio::test]
async fn an_offset_past_the_end_explains_the_range() {
    let fixture = Fixture::new();
    fixture.write_file("a.txt", "one\ntwo\n");

    let result = fixture
        .call("read", json!({ "filePath": "a.txt", "offset": 99 }))
        .await;

    assert!(!result.ok);
    assert!(result.output.contains("out of range"), "got {}", result.output);
    assert!(result.output.contains("2 lines"), "got {}", result.output);
}

/// A typo otherwise costs a whole round trip, with no way to discover the real
/// name.
#[tokio::test]
async fn a_missing_file_suggests_near_misses() {
    let fixture = Fixture::new();
    fixture.write_file("config.json", "{}");

    let result = fixture.call("read", json!({ "filePath": "config" })).await;

    assert!(!result.ok);
    assert!(
        result.output.contains("config.json"),
        "no suggestion offered: {}",
        result.output
    );
}

#[tokio::test]
async fn a_binary_file_is_refused_rather_than_mangled() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root.join("data.bin"), [0u8, 1, 2, 3, 0, 5]).unwrap();

    let result = fixture.call("read", json!({ "filePath": "data.bin" })).await;
    assert!(!result.ok);
    assert!(result.output.contains("binary"), "got {}", result.output);
}

/// Content sniffing, not just the extension: a `.txt` full of NUL bytes is
/// still binary.
#[tokio::test]
async fn binary_content_is_detected_by_sniffing() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root.join("sneaky.txt"), [b'a', 0, b'b', 0]).unwrap();

    let result = fixture.call("read", json!({ "filePath": "sneaky.txt" })).await;
    assert!(!result.ok);
    assert!(result.output.contains("binary"));
}

#[tokio::test]
async fn reading_a_directory_lists_it() {
    let fixture = Fixture::new();
    fixture.write_file("src/a.rs", "");
    fixture.write_file("src/b.rs", "");
    std::fs::create_dir_all(fixture.root.join("src/nested")).unwrap();

    let result = fixture.call("read", json!({ "filePath": "src" })).await;

    assert!(result.ok, "{}", result.output);
    assert!(result.output.contains("a.rs"));
    // A trailing slash so a folder is distinguishable without a second call.
    assert!(result.output.contains("nested/"), "got {}", result.output);
}

// ── write ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn writing_creates_the_file_and_its_parents() {
    let fixture = Fixture::new();

    let result = fixture
        .call(
            "write",
            json!({ "filePath": "deep/nested/new.txt", "content": "hello\n" }),
        )
        .await;

    assert!(result.ok, "{}", result.output);
    assert!(result.output.contains("Created"));
    assert_eq!(fixture.read_file("deep/nested/new.txt"), "hello\n");
}

#[tokio::test]
async fn rewriting_reports_that_it_replaced_something() {
    let fixture = Fixture::new();
    fixture.write_file("a.txt", "old\n");

    let result = fixture
        .call("write", json!({ "filePath": "a.txt", "content": "new\n" }))
        .await;

    assert!(result.output.contains("Wrote"), "got {}", result.output);
    assert_eq!(fixture.read_file("a.txt"), "new\n");
}

// ── edit, and the read-before-write gate ────────────────────────────────

/// The safety property that matters most here: an edit is a claim about
/// current contents, and a model that has not read the file is guessing.
#[tokio::test]
async fn editing_an_unread_file_is_refused() {
    let fixture = Fixture::new();
    fixture.write_file("a.txt", "hello world\n");

    let result = fixture
        .call(
            "edit",
            json!({ "filePath": "a.txt", "oldString": "world", "newString": "there" }),
        )
        .await;

    assert!(!result.ok);
    assert!(
        result.output.contains("has not been read"),
        "got {}",
        result.output
    );
    // And nothing was touched.
    assert_eq!(fixture.read_file("a.txt"), "hello world\n");
}

#[tokio::test]
async fn editing_after_reading_works() {
    let fixture = Fixture::new();
    fixture.write_file("a.txt", "hello world\n");

    fixture.call("read", json!({ "filePath": "a.txt" })).await;
    let result = fixture
        .call(
            "edit",
            json!({ "filePath": "a.txt", "oldString": "world", "newString": "there" }),
        )
        .await;

    assert!(result.ok, "{}", result.output);
    assert_eq!(fixture.read_file("a.txt"), "hello there\n");
}

/// A file read ten minutes ago and changed since is no better than one never
/// read.
#[tokio::test]
async fn editing_a_file_changed_since_reading_is_refused() {
    let fixture = Fixture::new();
    fixture.write_file("a.txt", "hello world\n");
    fixture.call("read", json!({ "filePath": "a.txt" })).await;

    // Something else changes it. The sleep is to guarantee a distinct mtime on
    // filesystems with coarse timestamp resolution.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fixture.write_file("a.txt", "something else entirely\n");

    let result = fixture
        .call(
            "edit",
            json!({ "filePath": "a.txt", "oldString": "something", "newString": "nothing" }),
        )
        .await;

    assert!(!result.ok);
    assert!(
        result.output.contains("changed on disk"),
        "got {}",
        result.output
    );
}

/// Writing a file makes it known-current, so a redundant read is not required.
#[tokio::test]
async fn a_file_just_written_can_be_edited_immediately() {
    let fixture = Fixture::new();

    fixture
        .call("write", json!({ "filePath": "a.txt", "content": "hello world\n" }))
        .await;
    let result = fixture
        .call(
            "edit",
            json!({ "filePath": "a.txt", "oldString": "world", "newString": "there" }),
        )
        .await;

    assert!(result.ok, "{}", result.output);
}

/// The model's own edit must not make its next edit look stale.
#[tokio::test]
async fn consecutive_edits_are_allowed() {
    let fixture = Fixture::new();
    fixture.write_file("a.txt", "one two three\n");
    fixture.call("read", json!({ "filePath": "a.txt" })).await;

    for (old, new) in [("one", "1"), ("two", "2"), ("three", "3")] {
        let result = fixture
            .call(
                "edit",
                json!({ "filePath": "a.txt", "oldString": old, "newString": new }),
            )
            .await;
        assert!(result.ok, "editing {old}: {}", result.output);
    }

    assert_eq!(fixture.read_file("a.txt"), "1 2 3\n");
}

#[tokio::test]
async fn an_ambiguous_edit_is_refused_with_a_usable_message() {
    let fixture = Fixture::new();
    fixture.write_file("a.txt", "x = 1\nx = 1\n");
    fixture.call("read", json!({ "filePath": "a.txt" })).await;

    let result = fixture
        .call(
            "edit",
            json!({ "filePath": "a.txt", "oldString": "x = 1", "newString": "x = 2" }),
        )
        .await;

    assert!(!result.ok);
    assert!(result.output.contains("Found 2 matches"), "got {}", result.output);
    // Unchanged: ambiguity never resolves by picking one.
    assert_eq!(fixture.read_file("a.txt"), "x = 1\nx = 1\n");
}

#[tokio::test]
async fn replace_all_handles_the_ambiguous_case() {
    let fixture = Fixture::new();
    fixture.write_file("a.txt", "x = 1\nx = 1\n");
    fixture.call("read", json!({ "filePath": "a.txt" })).await;

    let result = fixture
        .call(
            "edit",
            json!({
                "filePath": "a.txt",
                "oldString": "x = 1",
                "newString": "x = 2",
                "replaceAll": true
            }),
        )
        .await;

    assert!(result.ok, "{}", result.output);
    assert_eq!(fixture.read_file("a.txt"), "x = 2\nx = 2\n");
}

/// A file's own conventions must survive an edit, or every edited file shows
/// up in a diff as entirely rewritten.
#[tokio::test]
async fn crlf_line_endings_survive_an_edit() {
    let fixture = Fixture::new();
    fixture.write_file("a.txt", "one\r\ntwo\r\nthree\r\n");
    fixture.call("read", json!({ "filePath": "a.txt" })).await;

    let result = fixture
        .call(
            "edit",
            json!({ "filePath": "a.txt", "oldString": "two", "newString": "2" }),
        )
        .await;

    assert!(result.ok, "{}", result.output);
    let after = fixture.read_file("a.txt");
    assert_eq!(after, "one\r\n2\r\nthree\r\n");
    assert!(!after.contains("two"));
}

#[tokio::test]
async fn a_byte_order_mark_survives_an_edit() {
    let fixture = Fixture::new();
    fixture.write_file("a.txt", "\u{feff}hello world\n");
    fixture.call("read", json!({ "filePath": "a.txt" })).await;

    let result = fixture
        .call(
            "edit",
            json!({ "filePath": "a.txt", "oldString": "world", "newString": "there" }),
        )
        .await;

    assert!(result.ok, "{}", result.output);
    let after = fixture.read_file("a.txt");
    assert!(after.starts_with('\u{feff}'), "the BOM was lost");
    assert!(after.contains("hello there"));
    // Exactly one, not two.
    assert_eq!(after.matches('\u{feff}').count(), 1);
}

#[tokio::test]
async fn editing_a_missing_file_reports_it_as_missing() {
    let fixture = Fixture::new();
    let result = fixture
        .call(
            "edit",
            json!({ "filePath": "nope.txt", "oldString": "a", "newString": "b" }),
        )
        .await;

    assert!(!result.ok);
    assert!(result.output.contains("does not exist"), "got {}", result.output);
}

// ── the permission surface ──────────────────────────────────────────────

/// Reading and writing must not share a permission key: approving a read
/// should never imply approving a write.
#[tokio::test]
async fn reading_and_writing_ask_under_different_keys() {

    let state = Arc::new(ReadState::new());
    let tools = file_tools(state, Arc::new(inertia_lsp::Lsp::with_launcher(inertia_lsp::testing::fake_launcher())));
    let args = json!({ "filePath": "a.txt" });

    let keys: Vec<String> = tools.iter().map(|t| t.permission(&args).key).collect();
    assert_eq!(keys, vec!["read", "edit", "edit"]);

    // A read grant generalises to everything; a write grant does not.
    assert_eq!(tools[0].permission(&args).always.as_deref(), Some("*"));
    assert_eq!(tools[1].permission(&args).always.as_deref(), Some("a.txt"));
}
