//! EDT-B4 step 2: import dialog model.

use redox_editor_assets::{ImportDialogModel, ImportDialogPhase, ImportPresetId};
use std::path::PathBuf;

#[test]
fn confirm_queues_jobs_and_cancel_before_start() {
    let root = std::env::temp_dir().join(format!(
        "import_dialog_{}",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(root.join("textures"));
    let file = root.join("textures/a.png");
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&[0, 0, 0, 13]);
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&4u32.to_be_bytes());
    png.extend_from_slice(&4u32.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    std::fs::write(&file, png).expect("write");

    let mut dialog =
        ImportDialogModel::begin_paths(vec![file.clone()], &root);
    assert!(matches!(dialog.phase, ImportDialogPhase::Preview(_)));
    dialog.set_preset(0, ImportPresetId::TextureNormal);
    let jobs = dialog.confirm();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].path, "textures/a.png");
    assert!(matches!(dialog.phase, ImportDialogPhase::Committed(_)));

    let mut cancel_dialog =
        ImportDialogModel::begin_paths(vec![PathBuf::from("/nope/missing.png")], &root);
    assert!(!cancel_dialog.cancel());
    cancel_dialog = ImportDialogModel::begin_paths(vec![file], &root);
    assert!(cancel_dialog.cancel());
    assert!(matches!(cancel_dialog.phase, ImportDialogPhase::Cancelled));
}