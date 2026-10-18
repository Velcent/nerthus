//! Stable defaults for the user's documents, projects, builds and caches.

use std::path::{Path, PathBuf};

/// The user's documents folder (or home), with Nerthus's storage underneath.
/// A missing XDG documents setting must never make storage launch-relative.
pub fn default_storage_folder() -> PathBuf {
    let Some(user_dirs) = directories::UserDirs::new() else {
        // Browsers have virtual paths and no system temporary directory.
        #[cfg(target_arch = "wasm32")]
        let base = PathBuf::from("/");
        #[cfg(not(target_arch = "wasm32"))]
        let base = std::env::temp_dir();
        return base.join("Nerthus");
    };
    storage_folder(user_dirs.home_dir(), user_dirs.document_dir())
}

fn storage_folder(home: &Path, documents: Option<&Path>) -> PathBuf {
    if let Some(documents) = documents.filter(|folder| folder.is_absolute()) {
        return documents.join("Nerthus");
    }
    let documents = home.join("Documents");
    let base = if documents.is_dir() { documents } else { home.to_path_buf() };
    base.join("Nerthus")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_xdg_documents_uses_home_independently_of_the_launch_directory() {
        let home = std::env::temp_dir().join(format!("nerthus-storage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        assert_eq!(storage_folder(&home, None), home.join("Nerthus"));
        let documents = home.join("Documents");
        std::fs::create_dir(&documents).unwrap();
        assert_eq!(storage_folder(&home, None), documents.join("Nerthus"));
        assert_eq!(storage_folder(&home, Some(Path::new("relative"))), documents.join("Nerthus"));
        let custom = home.join("Custom Documents");
        assert_eq!(storage_folder(&home, Some(&custom)), custom.join("Nerthus"));
        std::fs::remove_dir_all(home).unwrap();
    }
}
