use std::path::{Path, PathBuf};

use agent_fw_catalog::CatalogScope;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct CatalogIndexPaths {
    root: PathBuf,
}

impl CatalogIndexPaths {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn catalog_root(&self) -> PathBuf {
        self.root.join(".agent-fw").join("indexes").join("catalog")
    }

    pub fn scope_path(&self, scope: &CatalogScope) -> PathBuf {
        self.catalog_root()
            .join(scope_hash(scope.tenant_id.as_str()))
            .join(scope_hash(scope.workspace_id.as_str()))
    }

    pub(crate) fn stale_marker_path(&self, scope: &CatalogScope) -> PathBuf {
        self.scope_path(scope).join(".stale")
    }
}

fn scope_hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..16])
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_core::{TenantId, WorkspaceId};

    #[test]
    fn scope_hash_is_hex_without_raw_identifier() {
        let paths = CatalogIndexPaths::new("/tmp/catalog-test");
        let scope = CatalogScope::new(
            TenantId::new_unchecked("tenant-visible"),
            WorkspaceId::new_unchecked("workspace-visible"),
        );

        let path = paths.scope_path(&scope);
        let path = path.to_string_lossy();

        assert!(!path.contains("tenant-visible"));
        assert!(!path.contains("workspace-visible"));
    }
}
