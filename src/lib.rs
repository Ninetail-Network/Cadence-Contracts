#![no_std]

#[cfg(not(target_arch = "wasm32"))]
#[macro_use]
extern crate std;

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, BytesN, Env,
};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DocumentStatus {
    Active,
    Revoked,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentRecord {
    pub issuer: Address,
    pub owner: Address,
    pub timestamp: u64,
    pub status: DocumentStatus,
}

#[contracttype]
pub enum DataKey {
    Document(BytesN<32>),
}

/// Enumeration of all error conditions that can occur within the ProofStell contract.
///
/// Each variant maps to a unique numeric code for Soroban client interoperability,
/// allowing callers to distinguish failure cases and implement appropriate recovery logic.
///
/// # Error Codes
/// | Variant                | Code | Description                                              |
/// |------------------------|------|----------------------------------------------------------|
/// | `DocumentNotFound`     | 1    | No record exists for the given document hash             |
/// | `AlreadyRegistered`    | 2    | A document with this hash has already been registered    |
/// | `OnlyIssuerCanRevoke`  | 3    | The caller is not the original issuer of the document    |
/// | `AlreadyRevoked`       | 4    | The document has already been revoked                    |
/// | `InvalidOwner`         | 5    | The provided owner address is not valid for this op      |
/// | `InvalidIssuer`        | 6    | The provided issuer address is not valid for this op     |
#[contracterror]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContractError {
    /// No record exists for the provided document hash. Code: 1
    DocumentNotFound = 1,
    /// A document with this hash is already registered. Code: 2
    AlreadyRegistered = 2,
    /// The caller is not the original issuer and cannot perform this action. Code: 3
    OnlyIssuerCanRevoke = 3,
    /// The document has already been revoked and cannot be revoked again. Code: 4
    AlreadyRevoked = 4,
    /// The provided owner address failed validation. Code: 5
    InvalidOwner = 5,
    /// The provided issuer address failed validation. Code: 6
    InvalidIssuer = 6,
}

#[contractevent(topics = ["register"], data_format = "vec")]
pub struct DocumentRegistered {
    #[topic]
    pub issuer: Address,
    pub owner: Address,
    pub document_hash: BytesN<32>,
}

#[contractevent(topics = ["revoke"], data_format = "vec")]
pub struct DocumentRevoked {
    #[topic]
    pub issuer: Address,
    pub document_hash: BytesN<32>,
}

#[contract]
pub struct ProofStellContract;

#[contractimpl]
impl ProofStellContract {
    /// Registers a new document on-chain, associating it with an issuer and owner.
    ///
    /// The issuer must authorize this call. Each document hash can only be registered once.
    ///
    /// # Arguments
    /// * `env`           - The Soroban environment
    /// * `issuer`        - Address of the entity registering the document (must authorize)
    /// * `owner`         - Address of the document's owner
    /// * `document_hash` - 32-byte unique hash identifying the document
    ///
    /// # Returns
    /// The newly created [`DocumentRecord`] with `DocumentStatus::Active`
    ///
    /// # Errors
    /// * [`ContractError::AlreadyRegistered`] — if a record already exists for this hash
    pub fn register_document(
        env: Env,
        issuer: Address,
        owner: Address,
        document_hash: BytesN<32>,
    ) -> Result<DocumentRecord, ContractError> {
        issuer.require_auth();

        let key = DataKey::Document(document_hash.clone());

        if env.storage().persistent().has(&key) {
            return Err(ContractError::AlreadyRegistered);
        }

        let record = DocumentRecord {
            issuer: issuer.clone(),
            owner,
            timestamp: env.ledger().timestamp(),
            status: DocumentStatus::Active,
        };

        env.storage().persistent().set(&key, &record);
        DocumentRegistered {
            issuer,
            owner: record.owner.clone(),
            document_hash,
        }
        .publish(&env);

        Ok(record)
    }

    /// Retrieves a document record by its hash from persistent storage.
    ///
    /// # Arguments
    /// * `env`           - The Soroban environment
    /// * `document_hash` - 32-byte hash identifying the document
    ///
    /// # Returns
    /// `Some(DocumentRecord)` if found, `None` otherwise
    pub fn get_document(env: Env, document_hash: BytesN<32>) -> Option<DocumentRecord> {
        let key = DataKey::Document(document_hash);
        env.storage().persistent().get(&key)
    }

    /// Checks whether a document exists and is currently active.
    ///
    /// # Arguments
    /// * `env`           - The Soroban environment
    /// * `document_hash` - 32-byte hash identifying the document
    ///
    /// # Returns
    /// `true` only if the document exists and has `DocumentStatus::Active`
    pub fn verify_document(env: Env, document_hash: BytesN<32>) -> bool {
        let key = DataKey::Document(document_hash);
        env.storage()
            .persistent()
            .get::<DataKey, DocumentRecord>(&key)
            .map_or(false, |record| record.status == DocumentStatus::Active)
    }

    /// Returns the status of a document, or an error if it does not exist.
    ///
    /// # Arguments
    /// * `env`           - The Soroban environment
    /// * `document_hash` - 32-byte hash identifying the document
    ///
    /// # Errors
    /// * [`ContractError::DocumentNotFound`] — if no record exists for the given hash
    pub fn get_document_status(
        env: Env,
        document_hash: BytesN<32>,
    ) -> Result<DocumentStatus, ContractError> {
        let key = DataKey::Document(document_hash);
        env.storage()
            .persistent()
            .get::<DataKey, DocumentRecord>(&key)
            .map(|record| record.status)
            .ok_or(ContractError::DocumentNotFound)
    }

    /// Checks whether a document exists in storage, regardless of its status.
    ///
    /// # Arguments
    /// * `env`           - The Soroban environment
    /// * `document_hash` - 32-byte hash identifying the document
    ///
    /// # Returns
    /// `true` if any record (active or revoked) is stored under this hash
    pub fn document_exists(env: Env, document_hash: BytesN<32>) -> bool {
        let key = DataKey::Document(document_hash);
        env.storage().persistent().has(&key)
    }

    /// Revokes a previously registered document, preventing future verification.
    ///
    /// Only the original issuer of the document may revoke it. A document that has
    /// already been revoked cannot be revoked again.
    ///
    /// # Arguments
    /// * `env`           - The Soroban environment
    /// * `issuer`        - Address of the original issuer (must authorize)
    /// * `document_hash` - 32-byte hash identifying the document to revoke
    ///
    /// # Returns
    /// The updated [`DocumentRecord`] with `DocumentStatus::Revoked`
    ///
    /// # Errors
    /// * [`ContractError::DocumentNotFound`]    — if no record exists for this hash
    /// * [`ContractError::OnlyIssuerCanRevoke`] — if the caller is not the original issuer
    /// * [`ContractError::AlreadyRevoked`]      — if the document is already revoked
    pub fn revoke_document(
        env: Env,
        issuer: Address,
        document_hash: BytesN<32>,
    ) -> Result<DocumentRecord, ContractError> {
        issuer.require_auth();

        let key = DataKey::Document(document_hash.clone());

        let mut record: DocumentRecord = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(ContractError::DocumentNotFound)?;

        if record.issuer != issuer {
            return Err(ContractError::OnlyIssuerCanRevoke);
        }

        if record.status == DocumentStatus::Revoked {
            return Err(ContractError::AlreadyRevoked);
        }

        record.status = DocumentStatus::Revoked;

        env.storage().persistent().set(&key, &record);
        DocumentRevoked {
            issuer,
            document_hash,
        }
        .publish(&env);

        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env};

    fn setup() -> (
        Env,
        ProofStellContractClient<'static>,
        Address,
        Address,
        BytesN<32>,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(ProofStellContract, ());
        let client = ProofStellContractClient::new(&env, &contract_id);
        let issuer = Address::generate(&env);
        let owner = Address::generate(&env);
        let document_hash = BytesN::from_array(&env, &[7; 32]);

        (env, client, issuer, owner, document_hash)
    }

    #[test]
    fn registers_and_verifies_document() {
        let (_env, client, issuer, owner, document_hash) = setup();

        let record = client.register_document(&issuer, &owner, &document_hash);

        assert_eq!(record.issuer, issuer);
        assert_eq!(record.owner, owner);
        assert_eq!(record.status, DocumentStatus::Active);
        assert!(client.verify_document(&document_hash));
    }

    #[test]
    fn returns_document_record() {
        let (_env, client, issuer, owner, document_hash) = setup();

        let record = client.register_document(&issuer, &owner, &document_hash);

        assert_eq!(client.get_document(&document_hash), Some(record));
    }

    #[test]
    fn revokes_document() {
        let (_env, client, issuer, owner, document_hash) = setup();

        client.register_document(&issuer, &owner, &document_hash);
        let record = client.revoke_document(&issuer, &document_hash);

        assert_eq!(record.status, DocumentStatus::Revoked);
        assert!(!client.verify_document(&document_hash));
    }

    #[test]
    fn prevents_duplicate_registration() {
        let (_env, client, issuer, owner, document_hash) = setup();

        client.register_document(&issuer, &owner, &document_hash);
        let err = client
            .try_register_document(&issuer, &owner, &document_hash)
            .unwrap_err()
            .unwrap();

        assert_eq!(err, ContractError::AlreadyRegistered);
    }

    #[test]
    fn prevents_non_issuer_revocation() {
        let (env, client, issuer, owner, document_hash) = setup();

        client.register_document(&issuer, &owner, &document_hash);

        let other = Address::generate(&env);
        let err = client
            .try_revoke_document(&other, &document_hash)
            .unwrap_err()
            .unwrap();

        assert_eq!(err, ContractError::OnlyIssuerCanRevoke);
    }

    #[test]
    fn prevents_double_revocation() {
        let (_env, client, issuer, owner, document_hash) = setup();

        client.register_document(&issuer, &owner, &document_hash);
        client.revoke_document(&issuer, &document_hash);

        let err = client
            .try_revoke_document(&issuer, &document_hash)
            .unwrap_err()
            .unwrap();

        assert_eq!(err, ContractError::AlreadyRevoked);
    }

    #[test]
    fn revoke_nonexistent_document_returns_not_found() {
        let (_env, client, issuer, _owner, document_hash) = setup();

        let err = client
            .try_revoke_document(&issuer, &document_hash)
            .unwrap_err()
            .unwrap();

        assert_eq!(err, ContractError::DocumentNotFound);
    }

    #[test]
    fn get_document_status_returns_not_found_for_missing_document() {
        let (_env, client, _issuer, _owner, document_hash) = setup();

        let err = client
            .try_get_document_status(&document_hash)
            .unwrap_err()
            .unwrap();

        assert_eq!(err, ContractError::DocumentNotFound);
    }

    #[test]
    fn get_document_status_returns_active_after_register() {
        let (_env, client, issuer, owner, document_hash) = setup();

        client.register_document(&issuer, &owner, &document_hash);
        let status = client.get_document_status(&document_hash);

        assert_eq!(status, DocumentStatus::Active);
    }

    #[test]
    fn get_document_status_returns_revoked_after_revoke() {
        let (_env, client, issuer, owner, document_hash) = setup();

        client.register_document(&issuer, &owner, &document_hash);
        client.revoke_document(&issuer, &document_hash);
        let status = client.get_document_status(&document_hash);

        assert_eq!(status, DocumentStatus::Revoked);
    }

    #[test]
    fn document_exists_returns_false_before_registration() {
        let (_env, client, _issuer, _owner, document_hash) = setup();

        assert!(!client.document_exists(&document_hash));
    }

    #[test]
    fn document_exists_returns_true_after_registration() {
        let (_env, client, issuer, owner, document_hash) = setup();

        client.register_document(&issuer, &owner, &document_hash);

        assert!(client.document_exists(&document_hash));
    }
}
