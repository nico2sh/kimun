use kimun_core::NoteVault;
use kimun_core::nfs::{NoteEntryData, VaultPath};
use kimun_core::note::NoteContentData;
use kimun_notes::cli::json_output::format_notes_with_content_as_json;
use tempfile::TempDir;

#[tokio::test]
async fn json_output_includes_required_fields() {
    let workspace_dir = TempDir::new().unwrap();
    let vault = NoteVault::new(workspace_dir.path()).await.unwrap();
    vault.validate_and_init().await.unwrap();

    let entries = vec![(
        NoteEntryData {
            path: VaultPath::note_path_from("test/note"),
            size: 1024,
            modified_secs: 1711454400,
        },
        NoteContentData {
            title: "Test Note".to_string(),
            hash: 0x123456789abcdef0,
        },
    )];

    let content_map = vec![(
        VaultPath::note_path_from("test/note"),
        "# Test Note\n\nContent here".to_string(),
    )];

    let json_str = format_notes_with_content_as_json(
        &vault,
        &entries,
        &content_map,
        "test-workspace",
        workspace_dir.path().to_str().unwrap(),
        Some("test query"),
        false, // is_listing
    )
    .unwrap();

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

    // Check nested metadata structure
    assert!(note["metadata"].is_object());
    assert!(note["metadata"]["tags"].is_array());
    assert!(note["metadata"]["links"].is_array());
    assert!(note["metadata"]["headers"].is_array());
}
