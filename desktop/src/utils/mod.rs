use base64::{engine::general_purpose, Engine};
use kimun_core::nfs::VaultPath;

pub mod keys;
pub mod md;
pub mod sparse_vector;

pub fn encode_path(path: &VaultPath) -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(path.to_string())
}

pub fn decode_path<S: AsRef<str>>(encoded: S) -> anyhow::Result<VaultPath> {
    let input = encoded.as_ref();
    let decoded = general_purpose::URL_SAFE_NO_PAD.decode(input)?;
    let decoded_string = String::from_utf8(decoded)?;
    let path = VaultPath::try_from(decoded_string)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        // Test basic path encoding and decoding
        let original_path = VaultPath::try_from("notes/test.md").unwrap();
        let encoded = encode_path(&original_path);
        let decoded = decode_path(&encoded).unwrap();
        assert_eq!(original_path, decoded);
    }

    #[test]
    fn test_empty_string() {
        // Empty String should default to empty path
        let encoded = "";
        let decoded = decode_path(encoded).unwrap();
        assert_eq!(VaultPath::try_from("").unwrap(), decoded);
    }

    #[test]
    fn test_encode_decode_with_special_characters() {
        // Test path with special characters that are allowed in VaultPath
        // Note: VaultPath doesn't allow: \ / : * ? " < > | [ ] ^ #
        // But allows spaces, &, !, @, $, %, +, etc.
        let original_path = VaultPath::try_from("notes/file with spaces & symbols!@$%.md").unwrap();
        let encoded = encode_path(&original_path);
        let decoded = decode_path(&encoded).unwrap();
        assert_eq!(original_path, decoded);
    }

    #[test]
    fn test_encode_decode_nested_path() {
        // Test deeply nested path
        let original_path = VaultPath::try_from("folder1/folder2/folder3/deep_file.md").unwrap();
        let encoded = encode_path(&original_path);
        let decoded = decode_path(&encoded).unwrap();
        assert_eq!(original_path, decoded);
    }

    #[test]
    fn test_encode_decode_unicode_characters() {
        // Test path with unicode characters
        let original_path = VaultPath::try_from("notes/文件名.md").unwrap();
        let encoded = encode_path(&original_path);
        let decoded = decode_path(&encoded).unwrap();
        assert_eq!(original_path, decoded);
    }

    #[test]
    fn test_decode_invalid_base64() {
        // Test decoding invalid base64 string
        let invalid_encoded = "invalid base64 string!";
        let result = decode_path(invalid_encoded);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_invalid_utf8() {
        // Test decoding base64 that doesn't represent valid UTF-8
        let invalid_utf8_bytes = vec![0xFF, 0xFE, 0xFD]; // Invalid UTF-8 sequence
        let encoded = general_purpose::URL_SAFE_NO_PAD.encode(&invalid_utf8_bytes);
        let result = decode_path(&encoded);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_invalid_vault_path() {
        // Test decoding valid UTF-8 that doesn't represent a valid VaultPath
        // VaultPath doesn't allow: \ / : * ? " < > | [ ] ^ #
        let invalid_path = "invalid/path:with*invalid?chars"; // Contains invalid chars : * ?
        let encoded = general_purpose::URL_SAFE_NO_PAD.encode(invalid_path);
        let result = decode_path(&encoded);
        assert!(result.is_err());
    }

    #[test]
    fn test_encode_decode_empty_path() {
        // Test that empty paths are valid (VaultPath allows empty paths)
        let original_path = VaultPath::try_from("").unwrap();
        let encoded = encode_path(&original_path);
        let decoded = decode_path(&encoded).unwrap();
        assert_eq!(original_path, decoded);
    }

    #[test]
    fn test_encode_produces_url_safe_base64() {
        // Test that encoding produces URL-safe base64 (no +, /, or = characters)
        let path = VaultPath::try_from("test/path.md").unwrap();
        let encoded = encode_path(&path);

        // URL-safe base64 should not contain +, /, or = characters
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains('='));
    }

    #[test]
    fn test_decode_with_string_types() {
        // Test that decode_path works with different string types
        let original_path = VaultPath::try_from("notes/test.md").unwrap();
        let encoded = encode_path(&original_path);

        // Test with &str
        let decoded1 = decode_path(&encoded).unwrap();
        assert_eq!(original_path, decoded1);

        // Test with String
        let decoded2 = decode_path(encoded.clone()).unwrap();
        assert_eq!(original_path, decoded2);

        // Test with &String
        let encoded_string = encoded;
        let decoded3 = decode_path(&encoded_string).unwrap();
        assert_eq!(original_path, decoded3);
    }
}
