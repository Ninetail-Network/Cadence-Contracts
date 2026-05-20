#![no_std]

use soroban_sdk::{contract, contractevent, contractimpl, contracttype, Address, BytesN, Env};

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
    pub fn register_document(
        env: Env,
        issuer: Address,
        owner: Address,
        document_hash: BytesN<32>,
    ) -> DocumentRecord {
        issuer.require_auth();

        let key = DataKey::Document(document_hash.clone());

        if env.storage().persistent().has(&key) {
            panic!("document already registered");
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

        record
    }

    pub fn get_document(env: Env, document_hash: BytesN<32>) -> Option<DocumentRecord> {
        let key = DataKey::Document(document_hash);

        env.storage().persistent().get(&key)
    }

    pub fn verify_document(env: Env, document_hash: BytesN<32>) -> bool {
        let key = DataKey::Document(document_hash);
        let record: Option<DocumentRecord> = env.storage().persistent().get(&key);

        matches!(
            record,
            Some(DocumentRecord {
                status: DocumentStatus::Active,
                ..
            })
        )
    }

    pub fn revoke_document(
        env: Env,
        issuer: Address,
        document_hash: BytesN<32>,
    ) -> DocumentRecord {
        issuer.require_auth();

        let key = DataKey::Document(document_hash.clone());
        let mut record: DocumentRecord = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic!("document not found"));

        if record.issuer != issuer {
            panic!("only issuer can revoke");
        }

        if record.status == DocumentStatus::Revoked {
            panic!("document already revoked");
        }

        record.status = DocumentStatus::Revoked;

        env.storage().persistent().set(&key, &record);
        DocumentRevoked {
            issuer,
            document_hash,
        }
        .publish(&env);

        record
    }
}
// test
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
    #[should_panic(expected = "document already registered")]
    fn prevents_duplicate_registration() {
        let (_env, client, issuer, owner, document_hash) = setup();

        client.register_document(&issuer, &owner, &document_hash);
        client.register_document(&issuer, &owner, &document_hash);
    }
}
