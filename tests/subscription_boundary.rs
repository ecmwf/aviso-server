use aviso_server::configuration::{NotificationBackendSettings, Settings};
use aviso_server::notification_backend::{
    JetStreamBackend, JetStreamConfig, NotificationBackend,
    in_memory::{InMemoryBackend, InMemoryConfig},
    replay::{BatchParams, StartAt},
};
use futures_util::StreamExt;
use std::sync::Arc;
use std::time::Duration;

async fn exercise(backend: Arc<dyn NotificationBackend>, base: String) {
    let topic = format!("{base}.match.yes");
    let excluded = format!("{base}.excluded.no");
    assert_eq!(
        backend
            .subscribe_to_topic(&topic)
            .await
            .unwrap()
            .history_end,
        0
    );
    assert_eq!(backend.history_end(&topic).await.unwrap(), 0);
    for subject in [&topic, &excluded, &topic] {
        backend.put_messages(subject, "{}".into()).await.unwrap();
        let h = backend.history_end(&topic).await.unwrap();
        assert_eq!(
            backend
                .subscribe_to_topic(&topic)
                .await
                .unwrap()
                .history_end,
            h
        );
    }
    // Deleted tail must still be part of the allocated sequence range.
    backend.delete_message(&base, 3).await.unwrap();
    let subscription = backend.subscribe_to_topic(&topic).await.unwrap();
    assert_eq!(subscription.history_end, 3);
    assert_eq!(backend.history_end(&topic).await.unwrap(), 3);
    backend.delete_message(&base, 1).await.unwrap();
    backend.delete_message(&base, 2).await.unwrap();
    let empty = backend.subscribe_to_topic(&topic).await.unwrap();
    assert_eq!(empty.history_end, 3, "nonzero empty stream");
    drop(empty);
    backend.put_messages(&topic, "{}".into()).await.unwrap();
    let mut live = subscription.stream;
    assert_eq!(live.next().await.unwrap().sequence, 4);
    for size in [1, 10] {
        let batch = backend
            .get_messages_batch(
                BatchParams::new(topic.clone(), size)
                    .with_sequence(0)
                    .with_end_sequence(3),
            )
            .await
            .unwrap();
        assert!(batch.messages.is_empty());
        assert!(!batch.has_more);
    }

    // Publications race creation, but the source boundary must partition them.
    let publisher = backend.clone();
    let published_topic = topic.clone();
    let published_excluded = excluded.clone();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let publisher_barrier = barrier.clone();
    let task = tokio::spawn(async move {
        publisher_barrier.wait().await;
        for i in 0..100 {
            let subject = if i % 2 == 0 {
                &published_topic
            } else {
                &published_excluded
            };
            publisher.put_messages(subject, "{}".into()).await.unwrap();
            tokio::task::yield_now().await;
        }
    });
    barrier.wait().await;
    let pattern = format!("{base}.*.yes");
    let subscription = backend.subscribe_to_topic(&pattern).await.unwrap();
    let h = subscription.history_end;
    task.await.unwrap();
    // A later matching publication makes the end of the live prefix observable.
    backend
        .put_messages(&topic, "sentinel".into())
        .await
        .unwrap();
    let end = backend.history_end(&topic).await.unwrap();
    let mut historical = Vec::new();
    for size in [1, 20] {
        let mut params = BatchParams::new(pattern.clone(), size)
            .with_sequence(0)
            .with_end_sequence(h);
        let mut sequences = Vec::new();
        let mut completed = false;
        for _ in 0..200 {
            // A moving matching/excluded tail must not extend the range.
            backend.put_messages(&topic, "after".into()).await.unwrap();
            backend
                .put_messages(&excluded, "after".into())
                .await
                .unwrap();
            let batch = backend.get_messages_batch(params.clone()).await.unwrap();
            assert!(batch.messages.iter().all(|m| m.sequence <= h));
            sequences.extend(batch.messages.iter().map(|m| m.sequence));
            if !batch.has_more {
                completed = true;
                break;
            }
            params = params.with_sequence(batch.next_sequence.unwrap());
            assert!(sequences.len() < 200, "replay must terminate");
        }
        assert!(completed, "bounded paging must terminate");
        if size == 1 {
            historical = sequences;
        } else {
            assert_eq!(sequences, historical);
        }
    }
    let mut live = subscription.stream;
    let mut combined = historical;
    loop {
        let message = live.next().await.unwrap();
        assert!(message.sequence > h);
        combined.push(message.sequence);
        if message.sequence == end {
            break;
        }
    }
    let expected: Vec<_> = std::iter::once(4)
        .chain((5..105).step_by(2))
        .chain(std::iter::once(105))
        .collect();
    assert_eq!(
        combined, expected,
        "atomic partition, ordered without duplicates"
    );

    for start in [
        StartAt::Sequence(u64::MAX),
        StartAt::Date(chrono::Utc::now() + chrono::Duration::days(1)),
    ] {
        let batch = backend
            .get_messages_batch(
                BatchParams::new(topic.clone(), 10)
                    .with_start_at(start)
                    .with_end_sequence(h),
            )
            .await
            .unwrap();
        assert!(batch.messages.is_empty());
        assert!(!batch.has_more);
    }
    backend.wipe_stream(&base).await.unwrap();
}

async fn overwritten_history(backend: Arc<dyn NotificationBackend>) {
    let base = format!("Overwrite{}", uuid::Uuid::new_v4().simple());
    let topic = format!("{base}.same");
    backend.put_messages(&topic, "old".into()).await.unwrap();
    let mut subscription = backend.subscribe_to_topic(&topic).await.unwrap();
    assert_eq!(subscription.history_end, 1);
    backend.put_messages(&topic, "new".into()).await.unwrap();
    let history = backend
        .get_messages_batch(
            BatchParams::new(topic.clone(), 20)
                .with_sequence(0)
                .with_end_sequence(subscription.history_end),
        )
        .await
        .unwrap();
    assert!(history.messages.is_empty());
    assert!(!history.has_more);
    // The surviving new message must not be discarded by a replay watermark.
    let live = subscription.stream.next().await.unwrap();
    assert_eq!(live.sequence, 2);
    assert_eq!(live.payload, "new");
    backend.wipe_stream(&base).await.unwrap();
}

#[tokio::test]
async fn in_memory_eviction_after_boundary() {
    let backend = InMemoryBackend::new(InMemoryConfig {
        max_history_per_topic: 1,
        max_topics: 10,
        enable_metrics: false,
    });
    tokio::time::timeout(
        Duration::from_secs(10),
        overwritten_history(Arc::new(backend)),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn in_memory_atomic_boundary() {
    let backend = InMemoryBackend::new(InMemoryConfig {
        max_history_per_topic: 1000,
        max_topics: 10,
        enable_metrics: false,
    });
    tokio::time::timeout(
        Duration::from_secs(30),
        exercise(
            Arc::new(backend),
            format!("Boundary{}", uuid::Uuid::new_v4().simple()),
        ),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn jetstream_initial_create_boundary() {
    if std::env::var("AVISO_RUN_NATS_TESTS").as_deref() != Ok("1") {
        return;
    }
    let base = format!("Boundary{}", uuid::Uuid::new_v4().simple());
    let settings: Settings = serde_json::from_value(serde_json::json!({
        "application": {"host": "127.0.0.1", "port": 0, "base_url": "http://localhost"},
        "notification_backend": {"kind": "in_memory"},
        "notification_schema": {"boundary": {
            "topic": {"base": base, "key_order": ["kind"]},
            "identifier": {"kind": {"type": "StringHandler", "required": true}},
            "storage_policy": {"allow_duplicates": true}
        }}
    }))
    .unwrap();
    settings.init_global_config();
    let settings: NotificationBackendSettings = serde_json::from_value(serde_json::json!({
        "kind": "jetstream", "jetstream": {"nats_url": std::env::var("NATS_URL").unwrap()}
    }))
    .unwrap();
    let backend = JetStreamBackend::new(JetStreamConfig::from_backend_settings(&settings).unwrap())
        .await
        .unwrap();
    assert_eq!(backend.client.server_info().version, "2.14.6");
    let backend = Arc::new(backend);
    tokio::time::timeout(Duration::from_secs(60), exercise(backend.clone(), base))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), overwritten_history(backend))
        .await
        .unwrap();
}
