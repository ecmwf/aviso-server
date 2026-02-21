use aviso_server::{
    configuration::{Settings, get_configuration},
    startup::Application,
};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;
// ============================================================================
// BENCHMARK CONFIGURATION
// ============================================================================

/// Number of notifications to send in each batch test
const NOTIFICATION_BATCH_SIZES: &[usize] = &[100, 1000];

/// Number of concurrent clients for concurrency tests
const CONCURRENT_CLIENT_COUNTS: &[usize] = &[1, 5, 10, 20];

/// How many times each test runs (minimum 10 required by Criterion)
/// Higher values = more accurate results but slower execution
const SAMPLE_SIZE: usize = 10;

/// How long each test runs in seconds
/// Higher values = more accurate results but slower execution
const MEASUREMENT_TIME_SECONDS: u64 = 5;

/// Timeout for individual HTTP requests in seconds
const REQUEST_TIMEOUT_SECONDS: u64 = 30;

/// How long to wait for server startup in milliseconds
const SERVER_STARTUP_DELAY_MS: u64 = 500;

/// Whether to run in-memory backend tests
/// Set to false to skip in-memory tests (useful for JetStream-only testing)
const ENABLE_IN_MEMORY_TESTS: bool = false;

/// Whether to run JetStream tests (requires NATS server running)
/// Set to false to skip JetStream tests (useful for in-memory-only testing)
const ENABLE_JETSTREAM_TESTS: bool = true;

/// Whether to run latency comparison tests
/// Set to false to skip single-notification latency tests
const ENABLE_LATENCY_TESTS: bool = true;

/// Stream name used for benchmark tests
/// This isolates benchmark data from production streams like "DISS" or "MARS"
const BENCHMARK_STREAM_NAME: &str = "BENCH";

// ============================================================================
// BENCHMARK APPLICATION WRAPPER
// ============================================================================

/// Wrapper for a running benchmark application instance
/// Manages the application lifecycle and provides cleanup capabilities
struct BenchmarkApp {
    /// HTTP address where the application is listening
    address: String,
    /// Handle to the running application (kept alive until dropped)
    _app_handle: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

impl BenchmarkApp {
    /// Spawn a new application instance with the specified backend
    /// Creates an isolated test environment with dedicated stream naming
    ///
    /// # Arguments
    /// * `backend_kind` - Type of backend to use ("in_memory" or "jetstream")
    ///
    /// # Returns
    /// * `Self` - Running application instance ready for testing
    async fn spawn_with_backend(backend_kind: &str) -> Self {
        println!(
            "🚀 Starting {} backend server for benchmarking...",
            backend_kind
        );

        // Load base configuration and modify it for benchmarking
        let mut configuration = get_configuration().expect("Failed to read configuration");

        // Configure the specific backend type
        configuration.notification_backend.kind = backend_kind.to_string();

        // Use random port to avoid conflicts with other test instances
        configuration.application.port = 0;

        // Patch notification schema to use benchmark stream instead of production streams
        // This prevents benchmark data from polluting production DISS/MARS streams
        if let Some(ref mut schema_map) = configuration.notification_schema {
            for (event_type, schema) in schema_map.iter_mut() {
                if let Some(ref mut topic_config) = schema.topic {
                    // Change base from "diss"/"mars" to "bench" for isolation
                    topic_config.base = BENCHMARK_STREAM_NAME.to_lowercase();
                    println!(
                        "📝 Patched {} schema to use '{}' stream for testing",
                        event_type, topic_config.base
                    );
                }
                if let Some(ref mut endpoint_config) = schema.endpoint {
                    // Also patch endpoint config if present
                    endpoint_config.base = BENCHMARK_STREAM_NAME.to_lowercase();
                }
            }
        }

        // Initialize global configuration with patched settings
        Settings::init_global_config(&configuration);

        let shutdown_token = CancellationToken::new();
        // Build and start the application
        let application = Application::build(configuration, shutdown_token.clone())
            .await
            .expect("Failed to build application");

        let address = format!("http://127.0.0.1:{}", application.port());
        let app_handle = tokio::spawn(application.run_until_stopped());

        // Wait for server to be ready
        tokio::time::sleep(Duration::from_millis(SERVER_STARTUP_DELAY_MS)).await;

        println!(
            "✅ {} backend ready at {} (using {} stream)",
            backend_kind, address, BENCHMARK_STREAM_NAME
        );

        Self {
            address,
            _app_handle: app_handle,
        }
    }

    /// Wipe all benchmark data to ensure clean test state
    /// This removes all data from the benchmark stream only, leaving production data intact
    ///
    /// # Returns
    /// * `Result<(), reqwest::Error>` - Success or HTTP error
    async fn wipe_benchmark_data(&self) -> Result<(), reqwest::Error> {
        let client = Client::new();

        println!(
            "🧹 Cleaning benchmark data from {} stream...",
            BENCHMARK_STREAM_NAME
        );

        // Wipe only the benchmark stream, not all data
        let response = client
            .delete(format!("{}/api/v1/admin/wipe/stream", self.address))
            .header("Content-Type", "application/json")
            .json(&json!({"stream_name": BENCHMARK_STREAM_NAME}))
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        if response.status().is_success() {
            println!("✅ Benchmark data wiped successfully");
        } else {
            eprintln!("⚠️  Failed to wipe benchmark data: {}", response.status());
        }

        Ok(())
    }
}

// ============================================================================
// TEST DATA GENERATION
// ============================================================================

/// Generate realistic test notification payloads
/// Creates varied data to simulate real-world usage patterns
/// All notifications use the benchmark stream to avoid production data pollution
///
/// # Arguments
/// * `id` - Unique identifier for this notification (affects field values)
///
/// # Returns
/// * `serde_json::Value` - Complete notification payload ready for sending
fn generate_notification_payload(id: usize) -> serde_json::Value {
    let domains = ["a", "b", "c", "d", "e"];
    let streams = ["enfo", "oper", "wave"];
    let events = ["dissemination", "mars"];

    json!({
        "event_type": events[id % 2],
        "request": {
            "target": format!("E{}", id % 10),
            "class": "od",
            "date": "20190810",
            "destination": format!("DEST{}", id % 100),
            "domain": domains[id % 5],
            "expver": format!("{:04}", (id % 9999) + 1),
            "step": (id % 240).to_string(),
            "stream": streams[id % 3],
            "time": format!("{:02}", (id % 24))
        },
        "payload": format!("/benchmark/data/file_{}.grib", id)
    })
}

// ============================================================================
// BENCHMARK EXECUTION FUNCTIONS
// ============================================================================

/// Send a batch of notifications using specified concurrency
/// Measures total time to send all notifications with given parallelism
///
/// # Arguments
/// * `client` - HTTP client for sending requests
/// * `app_address` - Base URL of the application
/// * `batch_size` - Total number of notifications to send
/// * `concurrent_clients` - Number of parallel clients to use
///
/// # Returns
/// * `Duration` - Total time taken to send all notifications
async fn send_notification_batch(
    client: &Client,
    app_address: &str,
    batch_size: usize,
    concurrent_clients: usize,
) -> Duration {
    let start = std::time::Instant::now();

    // Distribute notifications evenly across clients
    let notifications_per_client = batch_size / concurrent_clients;
    let remaining = batch_size % concurrent_clients;

    let mut tasks = Vec::new();

    // Spawn concurrent client tasks
    for client_id in 0..concurrent_clients {
        let client = client.clone();
        let app_address = app_address.to_string();

        // Some clients get one extra notification if batch_size doesn't divide evenly
        let notifications_for_client = if client_id < remaining {
            notifications_per_client + 1
        } else {
            notifications_per_client
        };

        let task = tokio::spawn(async move {
            // Each client sends its assigned notifications sequentially
            for i in 0..notifications_for_client {
                let notification_id = client_id * notifications_per_client + i;
                let payload = generate_notification_payload(notification_id);

                // Send notification with timeout protection
                let _result = client
                    .post(format!("{}/api/v1/notification", app_address))
                    .header("Content-Type", "application/json")
                    .json(&payload)
                    .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
                    .send()
                    .await;

                // Note: We ignore individual failures to focus on throughput measurement
                // In a real application, you'd want to handle these errors
            }
        });

        tasks.push(task);
    }

    // Wait for all clients to complete
    for task in tasks {
        let _ = task.await; // Ignore join errors for benchmark simplicity
    }

    start.elapsed()
}

// ============================================================================
// CRITERION BENCHMARK CONFIGURATION
// ============================================================================

/// Configure a benchmark group with consistent settings
/// Applies standard configuration to ensure reliable, comparable results
///
/// # Arguments
/// * `group` - Mutable reference to the benchmark group to configure
fn configure_benchmark_group(
    group: &mut criterion::BenchmarkGroup<criterion::measurement::WallTime>,
) {
    group
        .sample_size(SAMPLE_SIZE)
        .measurement_time(Duration::from_secs(MEASUREMENT_TIME_SECONDS))
        .warm_up_time(Duration::from_secs(2))
        .noise_threshold(0.02); // Ignore changes smaller than 2%
}

// ============================================================================
// BACKEND-SPECIFIC BENCHMARK FUNCTIONS
// ============================================================================

/// Generic benchmark function for any backend type
/// Runs comprehensive performance tests including batch sizes and concurrency
///
/// # Arguments
/// * `c` - Criterion instance for running benchmarks
/// * `backend_name` - Human-readable name for the backend (for reporting)
/// * `backend_kind` - Technical backend type ("in_memory" or "jetstream")
fn benchmark_backend(c: &mut Criterion, backend_name: &str, backend_kind: &str) {
    let rt = Runtime::new().unwrap();

    // Spawn application and clean any existing benchmark data
    let app = rt.block_on(async {
        let app = BenchmarkApp::spawn_with_backend(backend_kind).await;

        // Clean any existing benchmark data for consistent starting state
        if let Err(e) = app.wipe_benchmark_data().await {
            eprintln!("⚠️  Warning: Failed to clean benchmark data: {}", e);
        }

        app
    });

    let client = Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .build()
        .unwrap();

    let mut group = c.benchmark_group(format!("{}_backend", backend_name));
    configure_benchmark_group(&mut group);

    println!(
        "\n📊 Testing {} Backend Performance",
        backend_name.to_uppercase()
    );
    println!("{}", "=".repeat(60));

    // Batch size tests (single client) - measures raw throughput
    println!("🔄 Running batch size tests (single client)...");
    for &batch_size in NOTIFICATION_BATCH_SIZES {
        group.throughput(Throughput::Elements(batch_size as u64));
        group.bench_with_input(
            BenchmarkId::new("batch_size", format!("{}_notifications", batch_size)),
            &batch_size,
            |b, &batch_size| {
                println!("  📈 Testing batch of {} notifications...", batch_size);
                b.to_async(&rt).iter(|| async {
                    let duration = send_notification_batch(
                        &client,
                        &app.address,
                        batch_size,
                        1, // Single client
                    )
                    .await;
                    std::hint::black_box(duration)
                });
            },
        );
    }

    // Concurrency tests - measures scalability with parallel clients
    println!("🔄 Running concurrency tests (1000 notifications)...");
    for &client_count in CONCURRENT_CLIENT_COUNTS {
        group.throughput(Throughput::Elements(1000));
        group.bench_with_input(
            BenchmarkId::new("concurrency", format!("{}_clients", client_count)),
            &client_count,
            |b, &client_count| {
                println!("  🔀 Testing with {} concurrent clients...", client_count);
                b.to_async(&rt).iter(|| async {
                    let duration =
                        send_notification_batch(&client, &app.address, 1000, client_count).await;
                    std::hint::black_box(duration)
                });
            },
        );
    }

    group.finish();
    println!("✅ {} backend tests completed!\n", backend_name);

    // Clean up benchmark data after tests
    rt.block_on(async {
        if let Err(e) = app.wipe_benchmark_data().await {
            eprintln!("⚠️  Warning: Failed to clean up benchmark data: {}", e);
        }
    });
}

/// Benchmark in-memory backend if enabled
/// Provides fast, volatile storage performance baseline
fn benchmark_in_memory_backend(c: &mut Criterion) {
    if !ENABLE_IN_MEMORY_TESTS {
        println!("⏭️  Skipping in-memory tests (disabled in configuration)");
        return;
    }

    benchmark_backend(c, "in_memory", "in_memory");
}

/// Benchmark JetStream backend if enabled
/// Provides persistent, distributed storage performance metrics
fn benchmark_jetstream_backend(c: &mut Criterion) {
    if !ENABLE_JETSTREAM_TESTS {
        println!("⏭️  Skipping JetStream tests (disabled in configuration)");
        return;
    }

    benchmark_backend(c, "jetstream", "jetstream");
}

/// Compare single-notification latency between backends
/// Measures the overhead of each backend for individual operations
fn benchmark_latency_comparison(c: &mut Criterion) {
    if !ENABLE_LATENCY_TESTS {
        println!("⏭️  Skipping latency tests (disabled in configuration)");
        return;
    }

    let rt = Runtime::new().unwrap();

    println!("🔄 Starting latency comparison tests...");

    // Spawn both backends if enabled
    let (in_memory_app, jetstream_app) = rt.block_on(async {
        let in_memory = if ENABLE_IN_MEMORY_TESTS {
            let app = BenchmarkApp::spawn_with_backend("in_memory").await;
            let _ = app.wipe_benchmark_data().await;
            Some(app)
        } else {
            None
        };

        let jetstream = if ENABLE_JETSTREAM_TESTS {
            let app = BenchmarkApp::spawn_with_backend("jetstream").await;
            let _ = app.wipe_benchmark_data().await;
            Some(app)
        } else {
            None
        };

        (in_memory, jetstream)
    });

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let mut group = c.benchmark_group("latency_comparison");
    configure_benchmark_group(&mut group);

    let payload = generate_notification_payload(0);

    // Benchmark in-memory latency if available
    if let Some(ref app) = in_memory_app {
        group.bench_function("in_memory_single_notification", |b| {
            b.to_async(&rt).iter(|| async {
                let _result = client
                    .post(format!("{}/api/v1/notification", app.address))
                    .header("Content-Type", "application/json")
                    .json(&payload)
                    .send()
                    .await;
            });
        });
    }

    // Benchmark JetStream latency if available
    if let Some(ref app) = jetstream_app {
        group.bench_function("jetstream_single_notification", |b| {
            b.to_async(&rt).iter(|| async {
                let _result = client
                    .post(format!("{}/api/v1/notification", app.address))
                    .header("Content-Type", "application/json")
                    .json(&payload)
                    .send()
                    .await;
            });
        });
    }

    group.finish();
    println!("✅ Latency comparison completed!");

    // Clean up
    rt.block_on(async {
        if let Some(app) = in_memory_app {
            let _ = app.wipe_benchmark_data().await;
        }
        if let Some(app) = jetstream_app {
            let _ = app.wipe_benchmark_data().await;
        }
    });
}

// ============================================================================
// CRITERION MAIN CONFIGURATION
// ============================================================================

// Configure which benchmarks to run based on feature flags
criterion_group!(
    benches,
    benchmark_in_memory_backend,
    benchmark_jetstream_backend,
    benchmark_latency_comparison
);
criterion_main!(benches);
