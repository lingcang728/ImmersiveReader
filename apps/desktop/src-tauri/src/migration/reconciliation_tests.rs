use super::reconcile_zhihu_archive;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    let path = std::env::temp_dir().join(format!("immersive-reconcile-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("test root must exist");
    path
}

fn sqlite(database: &Path, sql: &str) {
    let output = Command::new("sqlite3")
        .arg(database)
        .arg(sql)
        .output()
        .expect("installed sqlite3 must start");
    assert!(
        output.status.success(),
        "sqlite failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn reconciliation_classifies_every_non_destructive_conflict() {
    let root = root();
    let database = root.join("zhihu.db");
    let output = root.join("知乎");
    fs::create_dir_all(output.join("Author")).expect("author directory must exist");
    fs::write(output.join(r"Author\ok.md"), "# ok").expect("valid markdown must write");
    fs::write(output.join(r"Author\orphan.md"), "# orphan").expect("orphan markdown must write");
    fs::write(output.join(r"Author\broken.md"), [0xff, 0xfe, 0xfd])
        .expect("invalid markdown must write");
    sqlite(
        &database,
        r#"
        CREATE TABLE items(id TEXT, author_id TEXT, author_name TEXT, url TEXT);
        CREATE TABLE task_items(task_id TEXT, item_id TEXT, status TEXT, output_path TEXT, updated_at INTEGER);
        INSERT INTO items VALUES
          ('ok','author','Author','https://example/ok'),
          ('missing','author','Author','https://example/missing'),
          ('none','author','Author','https://example/none'),
          ('outside','author','Author','https://example/outside');
        INSERT INTO task_items VALUES
          ('t1','ok','success','Author/ok.md',1),
          ('t2','ok','success','Author/ok.md',2),
          ('t1','missing','success','Author/missing.md',1),
          ('t1','none','success',NULL,1),
          ('t1','outside','success','../outside.md',1);
        "#,
    );

    let report = reconcile_zhihu_archive(Path::new("sqlite3"), &database, &output)
        .expect("reconciliation must succeed");
    let kinds = report
        .issues
        .iter()
        .map(|issue| issue.kind.clone())
        .collect::<BTreeSet<_>>();

    for expected in [
        "db-only",
        "file-only",
        "missing-file",
        "duplicate-success-path",
        "path-conflict",
        "manifest-missing",
        "provenance-missing",
        "unparseable-markdown",
        "multiple-candidates-for-source-item",
    ] {
        assert!(kinds.contains(expected), "missing category: {expected}");
    }
    assert!(report.unresolved_count >= 9);
    fs::remove_dir_all(root).expect("fixture must be removed");
}
