use super::tests::{
    build, declared, filled, key, mint, policy, refusal_kind, Ix, NOW, REWARDS_PROGRAM, SIGNER,
};
use super::*;

/// The throwaway `seed` keypair the host generates per Rewards round.
fn seed_key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[21u8; 32])
}

/// Rewards `CreateMerkleDistribution` (ix 5): signers are the payer and
/// authority (both the index-1 account) and the seed; it funds the vault from
/// the account's TINY token account by a Token-2022 CPI.
fn create_distribution() -> Ix {
    Ix {
        program: key(REWARDS_PROGRAM),
        accounts: vec![
            seed_key().verifying_key().to_bytes(),
            key(SIGNER),
            filled(3),
            filled(1),
            key(&mint()),
            key(TOKEN_2022_PROGRAM),
            key(SYSTEM_PROGRAM),
        ],
        data: vec![5, 0xaa],
    }
}

/// `tx` with signature slot `slot` set to `signature`.
fn with_signature(tx: &[u8], slot: usize, signature: &[u8; 64]) -> Vec<u8> {
    let decoded = decode_transaction(tx).unwrap();
    let mut out = tx.to_vec();
    let at = decoded.signatures_offset + slot * 64;
    out[at..at + 64].copy_from_slice(signature);
    out
}

fn seed_signed(tx: &[u8]) -> Vec<u8> {
    use ed25519_dalek::Signer as _;
    let message = decode_transaction(tx).unwrap().message;
    with_signature(tx, 1, &seed_key().sign(&message).to_bytes())
}

#[test]
fn a_co_signed_rewards_distribution_is_accepted_once_the_seed_has_signed() {
    let unsigned = build(key(SIGNER), 2, &[create_distribution()], None);
    let decoded = decode_transaction(&unsigned).unwrap();
    assert_eq!(
        base58(&decoded.account_keys[1]),
        base58(&seed_key().verifying_key().to_bytes())
    );
    // The seed has not signed yet.
    let refused = summarize(&decoded, SIGNER).unwrap_err();
    assert_eq!(refused.kind, RefusalKind::TransactionNotAllowed);
    assert!(
        refused.message.contains("has not signed"),
        "{}",
        refused.message
    );

    let signed = seed_signed(&unsigned);
    let summary = summarize(&decode_transaction(&signed).unwrap(), SIGNER).unwrap();
    assert_eq!(summary.signers.len(), 2);
    assert_eq!(summary.signers[0], SIGNER);
    assert_eq!(summary.programs, vec![REWARDS_PROGRAM.to_string()]);
    // The vault is funded by CPI: nothing visible, so the declared debit is
    // what the caps (1000 per transaction) apply to.
    assert!(summary.totals.is_empty());
    let check =
        |debits: &[MintTotal]| evaluate_program(&policy(), "solana", 1, &summary, debits, &[], NOW);
    assert_eq!(check(&declared(&mint(), 1_000)), Ok(()));
    assert_eq!(
        refusal_kind(check(&declared(&mint(), 5_000))),
        RefusalKind::PerTransaction
    );
}

#[test]
fn a_co_signature_that_does_not_verify_is_refused() {
    let unsigned = build(key(SIGNER), 2, &[create_distribution()], None);
    // Signed over other bytes, and garbage.
    let other = {
        use ed25519_dalek::Signer as _;
        seed_key().sign(b"something else").to_bytes()
    };
    for signature in [other, [0x11; 64]] {
        let tx = with_signature(&unsigned, 1, &signature);
        let refused = summarize(&decode_transaction(&tx).unwrap(), SIGNER).unwrap_err();
        assert_eq!(refused.kind, RefusalKind::TransactionNotAllowed);
        assert!(
            refused.message.contains("does not verify"),
            "{}",
            refused.message
        );
    }
}

#[test]
fn the_accounts_signature_fills_its_own_slot_and_completes_the_transaction() {
    use ed25519_dalek::Signer as _;
    let tx = seed_signed(&build(key(SIGNER), 2, &[create_distribution()], None));
    let decoded = decode_transaction(&tx).unwrap();
    // Stand-in for the module: any key's signature lands in the account's slot.
    let ours = ed25519_dalek::SigningKey::from_bytes(&[33u8; 32]);
    let signature = ours.sign(&decoded.message).to_bytes();
    let signed = attach_signature(&tx, &decoded, &key(SIGNER), &signature).unwrap();
    let after = decode_transaction(&signed).unwrap();
    assert_eq!(after.signatures[0], signature);
    assert_eq!(
        after.signatures[1], decoded.signatures[1],
        "the seed's stays"
    );
    assert_eq!(after.message, decoded.message);
    assert!(signature_is_valid(
        &key(&base58(&seed_key().verifying_key().to_bytes())),
        &after.message,
        &after.signatures[1]
    ));
    assert!(signature_is_valid(
        &ours.verifying_key().to_bytes(),
        &after.message,
        &after.signatures[0]
    ));
    // A key holding no signer slot cannot be placed.
    assert!(attach_signature(&tx, &decoded, &filled(3), &signature).is_err());
}
