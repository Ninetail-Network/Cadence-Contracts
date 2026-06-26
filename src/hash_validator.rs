use std::prelude::v1::*;

#[derive(Debug)]
pub enum ValidationError {
    WrongLength { expected: usize, actual: usize },
    InvalidCharacter { position: usize, character: char },
    EmptyHash,
    /// Hash algorithm is not supported for contract submission (only SHA-256 is accepted).
    UnsupportedAlgorithm,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HashAlgorithm {
    SHA256,
    SHA512,
}

pub struct HashValidator;

impl HashValidator {
    pub fn normalize(hash: &str) -> String {
        hash.trim().to_lowercase()
    }

    pub fn validate_sha256(hash: &str) -> Result<(), ValidationError> {
        Self::validate_with_length(hash, 64)
    }

    pub fn validate_sha512(hash: &str) -> Result<(), ValidationError> {
        Self::validate_with_length(hash, 128)
    }

    /// Validate that a hash is a canonical SHA-256 hex string suitable for
    /// contract submission. SHA-512 and all other lengths are explicitly rejected.
    ///
    /// Returns the normalized (lowercase, trimmed) hex string on success.
    pub fn validate_for_contract(hash: &str) -> Result<String, ValidationError> {
        let normalized = Self::normalize(hash);

        // Reject SHA-512 explicitly before falling through to the length check.
        if normalized.len() == 128 {
            return Err(ValidationError::UnsupportedAlgorithm);
        }

        Self::validate_with_length(&normalized, 64)?;
        Ok(normalized)
    }

    /// Convert a validated 64-character SHA-256 hex string to a 32-byte array.
    ///
    /// The input must already be a valid lowercase hex string of exactly 64 characters.
    /// Call [`validate_for_contract`] first to ensure the input is well-formed.
    pub fn hex_to_bytes32(hex: &str) -> Result<[u8; 32], ValidationError> {
        Self::validate_with_length(hex, 64)?;
        let mut bytes = [0u8; 32];
        for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
            let hi = Self::hex_nibble(chunk[0]).ok_or(ValidationError::InvalidCharacter {
                position: i * 2,
                character: chunk[0] as char,
            })?;
            let lo = Self::hex_nibble(chunk[1]).ok_or(ValidationError::InvalidCharacter {
                position: i * 2 + 1,
                character: chunk[1] as char,
            })?;
            bytes[i] = (hi << 4) | lo;
        }
        Ok(bytes)
    }

    fn hex_nibble(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        }
    }

    fn validate_with_length(hash: &str, expected_len: usize) -> Result<(), ValidationError> {
        let normalized = Self::normalize(hash);

        if normalized.is_empty() {
            return Err(ValidationError::EmptyHash);
        }

        let actual_len = normalized.len();
        if actual_len != expected_len {
            return Err(ValidationError::WrongLength {
                expected: expected_len,
                actual: actual_len,
            });
        }

        for (idx, ch) in normalized.chars().enumerate() {
            let is_hex = matches!(ch, '0'..='9' | 'a'..='f');
            if !is_hex {
                return Err(ValidationError::InvalidCharacter {
                    position: idx,
                    character: ch,
                });
            }
        }

        Ok(())
    }

    pub fn detect_algorithm(hash: &str) -> Option<HashAlgorithm> {
        let normalized = Self::normalize(hash);
        match normalized.len() {
            64 => Some(HashAlgorithm::SHA256),
            128 => Some(HashAlgorithm::SHA512),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Resolve ambiguous panic macro from glob import.
    use std::panic;

    fn sample_sha256() -> &'static str {
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    }

    fn sample_sha512() -> &'static str {
        "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce\
         47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
    }

    #[test]
    fn normalize_trims_and_lowercases() {
        let input = "  ABCdef123  ";
        let normalized = HashValidator::normalize(input);
        assert_eq!(normalized, "abcdef123");
    }

    #[test]
    fn sha256_valid_hash_passes() {
        assert!(HashValidator::validate_sha256(sample_sha256()).is_ok());
    }

    #[test]
    fn sha512_valid_hash_passes() {
        assert!(HashValidator::validate_sha512(sample_sha512()).is_ok());
    }

    #[test]
    fn wrong_length_error_for_63_char_hash() {
        let hash = "a".repeat(63);
        match HashValidator::validate_sha256(&hash) {
            Err(ValidationError::WrongLength { expected, actual }) => {
                assert_eq!(expected, 64);
                assert_eq!(actual, 63);
            }
            other => panic!("expected WrongLength error, got {:?}", other),
        }
    }

    #[test]
    fn empty_hash_errors() {
        match HashValidator::validate_sha256("") {
            Err(ValidationError::EmptyHash) => {}
            other => panic!("expected EmptyHash error, got {:?}", other),
        }
    }

    #[test]
    fn uppercase_hash_passes_after_normalization() {
        let upper = sample_sha256().to_uppercase();
        let normalized = HashValidator::normalize(&upper);
        assert!(HashValidator::validate_sha256(&normalized).is_ok());
    }

    #[test]
    fn invalid_character_reports_position() {
        let mut hash = sample_sha256().to_string();
        hash.replace_range(10..11, "g"); // 'g' is not a valid hex digit

        match HashValidator::validate_sha256(&hash) {
            Err(ValidationError::InvalidCharacter {
                position,
                character,
            }) => {
                assert_eq!(position, 10);
                assert_eq!(character, 'g');
            }
            other => panic!("expected InvalidCharacter error, got {:?}", other),
        }
    }

    #[test]
    fn detect_algorithm_identifies_sha256() {
        let algo = HashValidator::detect_algorithm(sample_sha256());
        assert_eq!(algo, Some(HashAlgorithm::SHA256));
    }

    #[test]
    fn detect_algorithm_identifies_sha512() {
        let algo = HashValidator::detect_algorithm(sample_sha512());
        assert_eq!(algo, Some(HashAlgorithm::SHA512));
    }

    #[test]
    fn detect_algorithm_returns_none_for_other_lengths() {
        let algo = HashValidator::detect_algorithm("abc123");
        assert_eq!(algo, None);
    }

    // ── validate_for_contract ─────────────────────────────────────────

    #[test]
    fn validate_for_contract_accepts_sha256() {
        let result = HashValidator::validate_for_contract(sample_sha256());
        assert_eq!(result.unwrap(), sample_sha256());
    }

    #[test]
    fn validate_for_contract_normalizes_uppercase_sha256() {
        let upper = sample_sha256().to_uppercase();
        let result = HashValidator::validate_for_contract(&upper);
        assert_eq!(result.unwrap(), sample_sha256());
    }

    #[test]
    fn validate_for_contract_rejects_sha512() {
        match HashValidator::validate_for_contract(sample_sha512()) {
            Err(ValidationError::UnsupportedAlgorithm) => {}
            other => panic!("expected UnsupportedAlgorithm, got {:?}", other),
        }
    }

    #[test]
    fn validate_for_contract_rejects_empty() {
        assert!(matches!(
            HashValidator::validate_for_contract(""),
            Err(ValidationError::EmptyHash)
        ));
    }

    // ── hex_to_bytes32 ────────────────────────────────────────────────

    #[test]
    fn hex_to_bytes32_converts_known_hash() {
        // SHA-256 of empty string
        let hex = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let bytes = HashValidator::hex_to_bytes32(hex).unwrap();
        assert_eq!(bytes[0], 0xe3);
        assert_eq!(bytes[1], 0xb0);
        assert_eq!(bytes[31], 0x55);
    }

    #[test]
    fn hex_to_bytes32_roundtrips_all_zero_hash() {
        let hex = "0".repeat(64);
        let bytes = HashValidator::hex_to_bytes32(&hex).unwrap();
        assert_eq!(bytes, [0u8; 32]);
    }

    #[test]
    fn hex_to_bytes32_rejects_wrong_length() {
        let hex = "a".repeat(63);
        assert!(matches!(
            HashValidator::hex_to_bytes32(&hex),
            Err(ValidationError::WrongLength { .. })
        ));
    }
}
