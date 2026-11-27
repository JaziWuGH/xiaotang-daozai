//! 插件私有存储的本地备份。
use super::*;
use std::io::Write;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Backup {
    id: String,
    data_version: u32,
    storage: Value,
}

impl LocalPlugins {
    fn storage_backup(&self, id: &str) -> Result<Backup> {
        let record = self.records.get(id).ok_or_else(|| fail("插件未安装"))?;
        let manifest = &record.package.as_ref()
            .filter(|package| package.manifest.has_capability("storage"))
            .ok_or_else(|| fail("此插件没有私有存储"))?.manifest;
        Ok(Backup { id: id.to_owned(), data_version: manifest.data_version, storage: record.storage.clone() })
    }

    pub(super) fn export_storage(&self, id: &str, folder: &Path) -> Result<PathBuf> {
        let bytes = serde_json::to_vec(&self.storage_backup(id)?)?;
        let time = chrono::Local::now().format("%Y%m%d-%H%M%S-%9f");
        let path = folder.join(format!("{id}-{time}.json"));
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        if let Err(error) = file.write_all(&bytes) {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(error.into());
        }
        Ok(path)
    }

    pub(super) fn import_storage(&mut self, id: &str, path: &Path) -> Result<()> {
        let current = self.storage_backup(id)?;
        let backup: Backup = serde_json::from_str(&bounded(path, 1024 * 1024)?)?;
        if backup.id != current.id || backup.data_version != current.data_version {
            return Err(fail("备份的插件 ID 或数据版本不匹配"));
        }
        if !script_settings::valid_storage(&backup.storage) {
            return Err(fail("备份的私有存储超过 256 KiB"));
        }
        let mut record = self.installed(id)?;
        record.storage = backup.storage;
        self.put(id, record)?;
        self.forget_view(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_backup_refuses_a_different_plugin_or_version() -> Result<()> {
        let root = std::env::temp_dir().join(format!("daozai-storage-backup-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        fs::create_dir_all(&root)?;
        let result = (|| -> Result<()> {
            let id = "example.sticky-notes";
            let mut host = LocalPlugins::open(root.join("installed"));
            host.install(&Path::new(env!("CARGO_MANIFEST_DIR")).join("spike/examples/sticky-notes/plugin.json"))?;
            let path = host.export_storage(id, &root)?;
            for (backup_id, version) in [("example.other", 1), (id, 2)] {
                fs::write(&path, json!({"id":backup_id,"dataVersion":version,"storage":{"items":[1]}}).to_string())?;
                assert!(host.import_storage(id, &path).is_err());
                assert!(host.records[id].storage.is_null());
            }
            let storage = json!({"items":[{"text":"两行\n便签"}]});
            fs::write(&path, json!({"id":id,"dataVersion":1,"storage":storage}).to_string())?;
            host.import_storage(id, &path)?;
            assert_eq!(host.records[id].storage, storage);
            assert_eq!(LocalPlugins::open(root.join("installed")).records[id].storage, storage);
            Ok(())
        })();
        let _ = fs::remove_dir_all(&root);
        result
    }
}
