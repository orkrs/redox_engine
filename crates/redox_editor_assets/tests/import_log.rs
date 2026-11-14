//! EDT-B4 step 4: import log model.

use redox_asset::AssetGuid;
use redox_editor_assets::{ImportEvent, ImportLogFilter, ImportLogModel};
use std::path::PathBuf;

#[test]
fn import_log_records_status_and_filter() {
    let mut log = ImportLogModel::new();
    let guid = AssetGuid(1);
    let path = PathBuf::from("/tmp/a.png");
    log.ingest(
        "a.png",
        guid,
        &ImportEvent::Started {
            path: path.clone(),
            guid,
        },
    );
    log.ingest(
        "a.png",
        guid,
        &ImportEvent::Failed {
            path,
            guid,
            message: "boom".into(),
        },
    );
    log.set_filter(ImportLogFilter::Problems);
    assert_eq!(log.visible().len(), 1);
    assert_eq!(log.retryable_paths(), vec!["a.png".to_string()]);
}