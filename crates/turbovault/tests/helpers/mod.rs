use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use turbovault_core::prelude::*;
use turbovault_parser::Parser;
use turbovault_tools::{FileTools, WriteMode};
use turbovault_vault::VaultManager;

pub struct TestVault {
    pub _temp_dir: TempDir,
    pub manager: Arc<VaultManager>,
    parser: Parser,
}

impl TestVault {
    pub async fn new() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = ServerConfig {
            vaults: vec![VaultConfig::builder("test", temp_dir.path())
                .build()
                .unwrap()],
            ..Default::default()
        };

        let manager = VaultManager::new(config).expect("Failed to create vault manager");
        manager.initialize().await.expect("Failed to initialize vault");

        let parser = Parser::new(temp_dir.path().to_path_buf());

        Self {
            _temp_dir: temp_dir,
            manager: Arc::new(manager),
            parser,
        }
    }

    pub fn file_tools(&self) -> FileTools {
        FileTools::new(self.manager.clone())
    }

    pub async fn write(&self, path: &str, content: &str) {
        self.file_tools()
            .write_file_with_mode(path, content, WriteMode::Overwrite)
            .await
            .expect("Failed to write file");
    }

    pub async fn read(&self, path: &str) -> String {
        self.file_tools()
            .read_file(path)
            .await
            .expect("Failed to read file")
    }

    pub fn parse(&self, path: &str, content: &str) -> VaultFile {
        self.parser
            .parse_file(&PathBuf::from(path), content)
            .expect("Failed to parse file")
    }

    pub async fn reinitialize(&self) {
        self.manager
            .initialize()
            .await
            .expect("Failed to reinitialize vault");
    }
}
