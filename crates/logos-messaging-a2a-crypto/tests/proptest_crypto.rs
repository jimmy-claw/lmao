//! Property-based tests for logos-messaging-a2a-crypto using proptest.
//!
//! These tests verify cryptographic invariants hold for arbitrary inputs,
//! complementing the hand-written unit tests with randomized exploration.

use logos_messaging_a2a_crypto::{AgentIdentity, EncryptedPayload, IntroBundle};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategy helpers
// ---------------------------------------------------------------------------

/// Generate a random 32-byte hex string (valid X25519 secret key material).
fn arb_secret_hex() -> impl Strategy<Value = String> {
    prop::collection::vec(any::<u8>(), 32).prop_map(|bytes| hex::encode(bytes))
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

proptest! {
    /// Encrypting then decrypting with the matching shared key always recovers
    /// the original plaintext, regardless of plaintext content or length.
    #[test]
    fn encrypt_decrypt_roundtrip(plaintext in prop::collection::vec(any::<u8>(), 0..4096)) {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();

        let key_ab = alice.shared_key(&bob.public);
        let key_ba = bob.shared_key(&alice.public);

        let encrypted = key_ab.encrypt(&plaintext).unwrap();
        let decrypted = key_ba.decrypt(&encrypted).unwrap();

        prop_assert_eq!(decrypted, plaintext);
    }

    /// A third party (Eve) who was not part of the ECDH exchange cannot
    /// decrypt messages encrypted for a different recipient.
    #[test]
    fn different_recipient_cannot_decrypt(plaintext in prop::collection::vec(any::<u8>(), 1..512)) {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();
        let eve = AgentIdentity::generate();

        let key_ab = alice.shared_key(&bob.public);
        let key_ae = alice.shared_key(&eve.public);

        let encrypted = key_ab.encrypt(&plaintext).unwrap();

        // Eve's shared key with Alice differs from Bob's, so decryption must fail.
        prop_assert!(key_ae.decrypt(&encrypted).is_err());
    }

    /// Tampering with any byte of the ciphertext causes AEAD authentication
    /// to fail, preventing undetected modification.
    #[test]
    fn tampered_ciphertext_fails_to_decrypt(
        plaintext in prop::collection::vec(any::<u8>(), 1..512),
        flip_pos_frac in 0.0f64..1.0,
    ) {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();
        let key = alice.shared_key(&bob.public);

        let encrypted = key.encrypt(&plaintext).unwrap();

        // Decode, flip one byte, re-encode
        let mut ct_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &encrypted.ciphertext,
        )
        .unwrap();

        let flip_idx = (flip_pos_frac * (ct_bytes.len() - 1) as f64) as usize;
        ct_bytes[flip_idx] ^= 0xff;

        let tampered = EncryptedPayload {
            nonce: encrypted.nonce.clone(),
            ciphertext: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &ct_bytes,
            ),
        };

        prop_assert!(key.decrypt(&tampered).is_err());
    }

    /// IntroBundle survives a JSON serialization roundtrip for any pubkey
    /// string, preserving all fields exactly.
    #[test]
    fn intro_bundle_json_roundtrip(pubkey in "[a-f0-9]{1,128}") {
        let bundle = IntroBundle::new(&pubkey);
        let json = serde_json::to_string(&bundle).unwrap();
        let recovered: IntroBundle = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(&recovered, &bundle);
        prop_assert_eq!(&recovered.agent_pubkey, &pubkey);
        prop_assert_eq!(&recovered.version, "1.0");
    }

    /// Encrypting the same plaintext twice with the same key always produces
    /// different ciphertexts (because nonces are random).
    #[test]
    fn nonce_uniqueness(plaintext in prop::collection::vec(any::<u8>(), 0..512)) {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();
        let key = alice.shared_key(&bob.public);

        let e1 = key.encrypt(&plaintext).unwrap();
        let e2 = key.encrypt(&plaintext).unwrap();

        // Nonces must differ (random 12-byte values).
        prop_assert_ne!(&e1.nonce, &e2.nonce);
        // Consequently, ciphertexts must also differ.
        prop_assert_ne!(&e1.ciphertext, &e2.ciphertext);
    }

    /// ECDH is symmetric: the shared key derived by Alice (using Bob's public
    /// key) and by Bob (using Alice's public key) are identical, so either
    /// side can encrypt and the other can decrypt.
    #[test]
    fn ecdh_symmetry(
        plaintext in prop::collection::vec(any::<u8>(), 0..1024),
        secret_a in arb_secret_hex(),
        secret_b in arb_secret_hex(),
    ) {
        let alice = AgentIdentity::from_hex(&secret_a).unwrap();
        let bob = AgentIdentity::from_hex(&secret_b).unwrap();

        // Alice encrypts for Bob
        let key_ab = alice.shared_key(&bob.public);
        let enc = key_ab.encrypt(&plaintext).unwrap();

        // Bob decrypts using the symmetric shared key
        let key_ba = bob.shared_key(&alice.public);
        let dec = key_ba.decrypt(&enc).unwrap();

        prop_assert_eq!(dec, plaintext);
    }

    /// An identity reconstructed from a hex-encoded secret always produces
    /// the same public key, and its derived shared keys are interchangeable
    /// with the original.
    #[test]
    fn from_hex_determinism(
        secret_hex in arb_secret_hex(),
        plaintext in prop::collection::vec(any::<u8>(), 0..256),
    ) {
        let id1 = AgentIdentity::from_hex(&secret_hex).unwrap();
        let id2 = AgentIdentity::from_hex(&secret_hex).unwrap();

        // Same secret → same public key
        prop_assert_eq!(id1.public_key_hex(), id2.public_key_hex());

        // Shared keys derived from reconstructed identities are interchangeable
        let peer = AgentIdentity::generate();
        let k1 = id1.shared_key(&peer.public);
        let k2 = id2.shared_key(&peer.public);

        let enc = k1.encrypt(&plaintext).unwrap();
        let dec = k2.decrypt(&enc).unwrap();
        prop_assert_eq!(dec, plaintext);
    }

    /// EncryptedPayload survives JSON serialization, and the deserialized
    /// form still decrypts correctly.
    #[test]
    fn encrypted_payload_json_roundtrip(plaintext in prop::collection::vec(any::<u8>(), 0..512)) {
        let alice = AgentIdentity::generate();
        let bob = AgentIdentity::generate();
        let key = alice.shared_key(&bob.public);

        let encrypted = key.encrypt(&plaintext).unwrap();
        let json = serde_json::to_string(&encrypted).unwrap();
        let recovered: EncryptedPayload = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(&recovered, &encrypted);

        // The deserialized payload must still decrypt correctly
        let decrypted = key.decrypt(&recovered).unwrap();
        prop_assert_eq!(decrypted, plaintext);
    }
}
