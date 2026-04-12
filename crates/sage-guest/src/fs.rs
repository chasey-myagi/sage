use anyhow::{Context, Result};
use sage_protocol::{FsEntry, FsListRequest, FsReadRequest, FsWriteRequest};
use std::path::Path;

pub async fn handle_read(req: &FsReadRequest) -> Result<Vec<u8>> {
    tokio::fs::read(&req.path)
        .await
        .with_context(|| format!("read {}", req.path))
}

pub async fn handle_write(req: &FsWriteRequest) -> Result<()> {
    if let Some(parent) = Path::new(&req.path).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&req.path, &req.data)
        .await
        .with_context(|| format!("write {}", req.path))
}

pub async fn handle_list(req: &FsListRequest) -> Result<Vec<FsEntry>> {
    let mut entries = Vec::new();
    let mut dir = tokio::fs::read_dir(&req.path).await?;
    while let Some(entry) = dir.next_entry().await? {
        let meta = entry.metadata().await?;
        entries.push(FsEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            size: meta.len(),
        });
    }
    Ok(entries)
}
