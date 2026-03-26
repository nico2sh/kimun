use kimun_notes::cli::json_output::format_notes_with_content_as_json;
use kimun_core::nfs::{NoteEntryData, VaultPath};
use kimun_core::note::NoteContentData;

#[test]
fn json_output_includes_required_fields() {
    let entries = vec![(
        NoteEntryData {
            path: VaultPath::note_path_from("test/note"),
            size: 1024,
            modified_secs: 1711454400,
        },
        NoteContentData {
            title: "Test Note".to_string(),
            hash: 0x123456789abcdef0,
        }
    )];

    let content_map = vec![
        (VaultPath::note_path_from("test/note"), "# Test Note\n\nContent here".to_string())
    ];

    let json_str = format_notes_with_content_as_json(
        &entries,
        &content_map,
        "test-workspace",
        "/path/to/workspace",
        Some("test query"),
        false
    ).unwrap();

    let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    // Check metadata
    assert_eq!(json["metadata"]["workspace"], "test-workspace");
    assert_eq!(json["metadata"]["total_results"], 1);
    assert!(json["metadata"]["generated_at"].is_string());

    // Check note fields
    let note = &json["notes"][0];
    assert_eq!(note["path"], "test/note.md");
    assert_eq!(note["title"], "Test Note");
    assert_eq!(note["content"], "# Test Note\n\nContent here");
    assert_eq!(note["size"], 1024);
    assert_eq!(note["modified"], 1711454400);
}
