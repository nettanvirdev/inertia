//! Reads a real workspace folder and reports what came back.
//!
//! A unit test proves the code agrees with itself; this proves it agrees with a
//! folder that real use has written, including one an older build produced.
//! Anything reported as unreadable is a record the app would quarantine.
//!
//!     cargo run -p inertia-store --example scan -- ~/Documents/Inertia

use inertia_store::collections;
use inertia_store::layout::{Collection, Document, Layout};

fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: scan <workspace folder>");
        std::process::exit(2);
    });
    let layout = Layout::new(&root);
    println!("workspace: {root}\n");

    println!("collections");
    for collection in Collection::ALL {
        let ids = collections::ids(&layout, *collection);
        let records = collections::list(&layout, *collection);
        let lost = ids.len().saturating_sub(records.len());
        let flag = if lost > 0 { "  <-- UNREADABLE" } else { "" };
        println!(
            "  {:<18} {:>3} on disk, {:>3} read{}",
            collection.key(),
            ids.len(),
            records.len(),
            flag
        );
        if lost > 0 {
            for id in &ids {
                if collections::get(&layout, *collection, id)
                    .ok()
                    .flatten()
                    .is_none()
                {
                    println!("      could not read: {id}");
                }
            }
        }
    }

    println!("\ndocuments");
    for document in Document::ALL {
        let path = layout.document(*document);
        if !path.exists() {
            println!("  {:<22} -", document.key());
            continue;
        }
        let value = collections::read_document(&layout, *document, serde_json::Value::Null);
        let summary = match &value {
            serde_json::Value::Null => "UNREADABLE".to_string(),
            serde_json::Value::Object(map) => format!("{} keys", map.len()),
            other => format!("{other}"),
        };
        println!("  {:<22} {summary}", document.key());
    }

    println!("\nsecrets");
    for row in inertia_store::secrets::list(&layout) {
        println!("  {:<24} {}", row["name"], row["hint"]);
    }
}
