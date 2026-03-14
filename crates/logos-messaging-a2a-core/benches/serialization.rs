use criterion::{black_box, criterion_group, criterion_main, Criterion};
use logos_messaging_a2a_core::{AgentCard, Message, Part, TaskState};
use logos_messaging_a2a_crypto::IntroBundle;

fn sample_agent_card() -> AgentCard {
    AgentCard {
        name: "echo-agent".to_string(),
        description: "Echoes messages back to the sender".to_string(),
        version: "0.1.0".to_string(),
        capabilities: vec![
            "text".to_string(),
            "echo".to_string(),
            "translate".to_string(),
        ],
        public_key: "02abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab"
            .to_string(),
        intro_bundle: Some(IntroBundle::new(
            "aabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccddaabbccdd",
        )),
    }
}

fn sample_message() -> Message {
    Message {
        role: "user".to_string(),
        parts: vec![
            Part::Text {
                text: "Hello, can you help me with a task?".to_string(),
            },
            Part::Text {
                text: "I need to translate this document from English to Spanish.".to_string(),
            },
        ],
    }
}

fn bench_agent_card_serialize(c: &mut Criterion) {
    let card = sample_agent_card();
    c.bench_function("agent_card_serialize", |b| {
        b.iter(|| serde_json::to_string(black_box(&card)).unwrap())
    });
}

fn bench_agent_card_deserialize(c: &mut Criterion) {
    let card = sample_agent_card();
    let json = serde_json::to_string(&card).unwrap();
    c.bench_function("agent_card_deserialize", |b| {
        b.iter(|| serde_json::from_str::<AgentCard>(black_box(&json)).unwrap())
    });
}

fn bench_task_state_serialize(c: &mut Criterion) {
    let states = [
        TaskState::Submitted,
        TaskState::Working,
        TaskState::InputRequired,
        TaskState::Completed,
        TaskState::Failed,
        TaskState::Cancelled,
    ];
    c.bench_function("task_state_serialize_all", |b| {
        b.iter(|| {
            for state in &states {
                serde_json::to_string(black_box(state)).unwrap();
            }
        })
    });
}

fn bench_task_state_deserialize(c: &mut Criterion) {
    let jsons = [
        "\"submitted\"",
        "\"working\"",
        "\"input_required\"",
        "\"completed\"",
        "\"failed\"",
        "\"cancelled\"",
    ];
    c.bench_function("task_state_deserialize_all", |b| {
        b.iter(|| {
            for json in &jsons {
                serde_json::from_str::<TaskState>(black_box(json)).unwrap();
            }
        })
    });
}

fn bench_part_serialize(c: &mut Criterion) {
    let part = Part::Text {
        text: "Hello, this is a sample text part for benchmarking serialization performance."
            .to_string(),
    };
    c.bench_function("part_serialize", |b| {
        b.iter(|| serde_json::to_string(black_box(&part)).unwrap())
    });
}

fn bench_part_deserialize(c: &mut Criterion) {
    let part = Part::Text {
        text: "Hello, this is a sample text part for benchmarking serialization performance."
            .to_string(),
    };
    let json = serde_json::to_string(&part).unwrap();
    c.bench_function("part_deserialize", |b| {
        b.iter(|| serde_json::from_str::<Part>(black_box(&json)).unwrap())
    });
}

fn bench_message_serialize(c: &mut Criterion) {
    let msg = sample_message();
    c.bench_function("message_serialize", |b| {
        b.iter(|| serde_json::to_string(black_box(&msg)).unwrap())
    });
}

fn bench_message_deserialize(c: &mut Criterion) {
    let msg = sample_message();
    let json = serde_json::to_string(&msg).unwrap();
    c.bench_function("message_deserialize", |b| {
        b.iter(|| serde_json::from_str::<Message>(black_box(&json)).unwrap())
    });
}

criterion_group!(
    benches,
    bench_agent_card_serialize,
    bench_agent_card_deserialize,
    bench_task_state_serialize,
    bench_task_state_deserialize,
    bench_part_serialize,
    bench_part_deserialize,
    bench_message_serialize,
    bench_message_deserialize,
);
criterion_main!(benches);
