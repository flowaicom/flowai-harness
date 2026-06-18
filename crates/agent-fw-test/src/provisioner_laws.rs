//! DatabaseProvisioner algebraic law test harnesses.
//!
//! # Laws
//!
//! - L1. Provision-List: After provision, list includes new environment
//! - L2. Deprovision-Removes: After deprovision, list excludes it
//! - L3. Name-Uniqueness: Two provisions with same name fail on second
//! - L5. Deprovision-NotFound: Deprovisioning non-existent returns NotFound
//!
//! Note: L4 (Connection-Valid) is integration-only, requires a real database.
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn my_provisioner_satisfies_laws() {
//!     let prov = MyProvisioner::new();
//!     agent_fw_test::provisioner_laws::test_all(&prov).await;
//! }
//! ```

use agent_fw_catalog::{
    DatabaseProvisioner, EnvironmentId, EnvironmentName, ProvisionRequest, ProvisioningError,
};

/// Run all deterministic DatabaseProvisioner laws (L1-L3, L5).
pub async fn test_all(prov: &dyn DatabaseProvisioner) {
    law_provision_appears_in_list(prov).await;
    law_deprovision_removes_from_list(prov).await;
    law_name_uniqueness(prov).await;
    law_deprovision_not_found(prov).await;
}

/// L1: After provision, list_environments includes the new environment.
pub async fn law_provision_appears_in_list(prov: &dyn DatabaseProvisioner) {
    let env = prov
        .provision(ProvisionRequest {
            name: EnvironmentName::new("law-l1-test"),
            parent_id: None,
            expires_at: None,
        })
        .await
        .expect("L1: provision should succeed");

    let envs = prov
        .list_environments()
        .await
        .expect("L1: list should succeed");

    assert!(
        envs.iter().any(|e| e.id == env.id),
        "L1 violated: provisioned environment not in list"
    );

    // Cleanup
    let _ = prov.deprovision(&env.id).await;
}

/// L2: After deprovision, list_environments excludes the environment.
pub async fn law_deprovision_removes_from_list(prov: &dyn DatabaseProvisioner) {
    let env = prov
        .provision(ProvisionRequest {
            name: EnvironmentName::new("law-l2-test"),
            parent_id: None,
            expires_at: None,
        })
        .await
        .expect("L2: provision should succeed");

    prov.deprovision(&env.id)
        .await
        .expect("L2: deprovision should succeed");

    let envs = prov
        .list_environments()
        .await
        .expect("L2: list should succeed");

    assert!(
        !envs.iter().any(|e| e.id == env.id),
        "L2 violated: deprovisioned environment still in list"
    );
}

/// L3: Two provisions with the same name — second fails with Conflict.
pub async fn law_name_uniqueness(prov: &dyn DatabaseProvisioner) {
    let env = prov
        .provision(ProvisionRequest {
            name: EnvironmentName::new("law-l3-unique"),
            parent_id: None,
            expires_at: None,
        })
        .await
        .expect("L3: first provision should succeed");

    let result = prov
        .provision(ProvisionRequest {
            name: EnvironmentName::new("law-l3-unique"),
            parent_id: None,
            expires_at: None,
        })
        .await;

    assert!(
        matches!(result, Err(ProvisioningError::Conflict(_))),
        "L3 violated: duplicate name should return Conflict, got: {result:?}"
    );

    // Cleanup
    let _ = prov.deprovision(&env.id).await;
}

/// L5: Deprovisioning a non-existent environment returns NotFound.
pub async fn law_deprovision_not_found(prov: &dyn DatabaseProvisioner) {
    let result = prov
        .deprovision(&EnvironmentId::new("nonexistent-env-999"))
        .await;

    assert!(
        matches!(result, Err(ProvisioningError::NotFound(_))),
        "L5 violated: deprovisioning non-existent should return NotFound, got: {result:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_interpreter::MockProvisioner;

    #[tokio::test]
    async fn mock_provisioner_satisfies_all_laws() {
        let prov = MockProvisioner::new();
        test_all(&prov).await;
    }
}
