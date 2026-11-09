use anyhow::Result;
use gpui::{AssetSource, SharedString};
use std::borrow::Cow;
use std::path::PathBuf;

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Ok(executable) = std::env::current_exe() {
            let mut directories = Vec::new();
            if let Some(bin) = executable.parent() {
                directories.push(bin.join("Resources"));
                if let Some(contents) = bin.parent() {
                    directories.push(contents.join("Resources"));
                }
            }
            for directory in directories {
                let resource = directory.join(path);
                if resource.is_file() {
                    return Ok(Some(Cow::Owned(std::fs::read(resource)?)));
                }
            }
        }
        let development = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join(path);
        if development.is_file() {
            return Ok(Some(Cow::Owned(std::fs::read(development)?)));
        }
        Ok(None)
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

pub fn resource_path(name: &str) -> String {
    format!("icons/{name}.svg")
}
