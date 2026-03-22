use criterion::{black_box, criterion_group, criterion_main, Criterion};
use logos_messaging_a2a_crypto::AgentIdentity;

fn bench_key_generation(c: &mut Criterion) {
    c.bench_function("key_generation", |b| b.iter(|| AgentIdentity::generate()));
}

fn bench_shared_key_derivation(c: &mut Criterion) {
    let alice = AgentIdentity::generate();
    let bob = AgentIdentity::generate();
    c.bench_function("shared_key_derivation", |b| {
        b.iter(|| alice.shared_key(black_box(&bob.public)))
    });
}

fn bench_encrypt_small(c: &mut Criterion) {
    let alice = AgentIdentity::generate();
    let bob = AgentIdentity::generate();
    let key = alice.shared_key(&bob.public);
    let plaintext = b"Hello, encrypted world!";
    c.bench_function("encrypt_small_23B", |b| {
        b.iter(|| key.encrypt(black_box(plaintext)).unwrap())
    });
}

fn bench_decrypt_small(c: &mut Criterion) {
    let alice = AgentIdentity::generate();
    let bob = AgentIdentity::generate();
    let key = alice.shared_key(&bob.public);
    let encrypted = key.encrypt(b"Hello, encrypted world!").unwrap();
    c.bench_function("decrypt_small_23B", |b| {
        b.iter(|| key.decrypt(black_box(&encrypted)).unwrap())
    });
}

fn bench_encrypt_1kb(c: &mut Criterion) {
    let alice = AgentIdentity::generate();
    let bob = AgentIdentity::generate();
    let key = alice.shared_key(&bob.public);
    let plaintext = vec![0xab_u8; 1024];
    c.bench_function("encrypt_1KB", |b| {
        b.iter(|| key.encrypt(black_box(&plaintext)).unwrap())
    });
}

fn bench_decrypt_1kb(c: &mut Criterion) {
    let alice = AgentIdentity::generate();
    let bob = AgentIdentity::generate();
    let key = alice.shared_key(&bob.public);
    let encrypted = key.encrypt(&vec![0xab_u8; 1024]).unwrap();
    c.bench_function("decrypt_1KB", |b| {
        b.iter(|| key.decrypt(black_box(&encrypted)).unwrap())
    });
}

fn bench_encrypt_64kb(c: &mut Criterion) {
    let alice = AgentIdentity::generate();
    let bob = AgentIdentity::generate();
    let key = alice.shared_key(&bob.public);
    let plaintext = vec![0xab_u8; 64 * 1024];
    c.bench_function("encrypt_64KB", |b| {
        b.iter(|| key.encrypt(black_box(&plaintext)).unwrap())
    });
}

fn bench_decrypt_64kb(c: &mut Criterion) {
    let alice = AgentIdentity::generate();
    let bob = AgentIdentity::generate();
    let key = alice.shared_key(&bob.public);
    let encrypted = key.encrypt(&vec![0xab_u8; 64 * 1024]).unwrap();
    c.bench_function("decrypt_64KB", |b| {
        b.iter(|| key.decrypt(black_box(&encrypted)).unwrap())
    });
}

fn bench_roundtrip_encrypt_decrypt(c: &mut Criterion) {
    let alice = AgentIdentity::generate();
    let bob = AgentIdentity::generate();
    let key_ab = alice.shared_key(&bob.public);
    let key_ba = bob.shared_key(&alice.public);
    let plaintext = b"Roundtrip benchmark message";
    c.bench_function("roundtrip_encrypt_decrypt", |b| {
        b.iter(|| {
            let encrypted = key_ab.encrypt(black_box(plaintext)).unwrap();
            key_ba.decrypt(black_box(&encrypted)).unwrap()
        })
    });
}

criterion_group!(
    benches,
    bench_key_generation,
    bench_shared_key_derivation,
    bench_encrypt_small,
    bench_decrypt_small,
    bench_encrypt_1kb,
    bench_decrypt_1kb,
    bench_encrypt_64kb,
    bench_decrypt_64kb,
    bench_roundtrip_encrypt_decrypt,
);
criterion_main!(benches);
