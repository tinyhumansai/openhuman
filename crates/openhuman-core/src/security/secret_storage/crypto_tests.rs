use super::*;

#[test]
fn roundtrip_encrypt_decrypt() {
    let key = [42u8; KEY_LEN];
    let plaintext = b"hello world";
    let blob = chacha20_encrypt(&key, plaintext).unwrap();
    let decrypted = chacha20_decrypt(&key, &blob).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn decrypt_wrong_key_fails() {
    let key1 = [1u8; KEY_LEN];
    let key2 = [2u8; KEY_LEN];
    let blob = chacha20_encrypt(&key1, b"secret").unwrap();
    assert!(chacha20_decrypt(&key2, &blob).is_err());
}

#[test]
fn decrypt_short_blob_fails() {
    let key = [0u8; KEY_LEN];
    assert!(chacha20_decrypt(&key, &[0u8; NONCE_LEN]).is_err());
}

#[test]
fn hex_roundtrip() {
    let data = vec![0xde, 0xad, 0xbe, 0xef];
    assert_eq!(hex_encode(&data), "deadbeef");
    assert_eq!(hex_decode("deadbeef").unwrap(), data);
}
