<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_light.svg">
    <img alt="Aviso Logo" src="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_light.svg">
  </picture>
</div>

<p align="center">
  <a href="https://github.com/ecmwf/codex/raw/refs/heads/main/ESEE">
    <img src="https://github.com/ecmwf/codex/raw/refs/heads/main/ESEE/foundation_badge.svg" alt="Foundation Badge">
  </a>
  <a href="https://github.com/ecmwf/codex/raw/refs/heads/main/Project%20Maturity">
    <img src="https://github.com/ecmwf/codex/raw/refs/heads/main/Project%20Maturity/emerging_badge.svg" alt="Maturity Badge">
  </a>
</p>

> [!IMPORTANT]  
> This software is **Emerging** and subject to ECMWF's guidelines on [Software Maturity](https://github.com/ecmwf/codex/raw/refs/heads/main/Project%20Maturity).

## What is Aviso Server?

Aviso Server is a specialized notification system that enables real-time monitoring and historical replay of data dissemination events. It's built for environments where timely notification of data availability is critical.

## Core Capabilities

### Real-time Event Streaming
- **Server-Sent Events (SSE):** Persistent connections for real-time notification delivery
- **CloudEvent Format:** Standardized event formatting for interoperability
- **Connection Management:** Automatic heartbeats, timeouts, and graceful reconnection

### Intelligent Pattern Matching
- **Wildcard Subscriptions:** Subscribe to notification patterns using flexible wildcards
- **Hybrid Filtering:** Efficient two-tier filtering combining backend optimization with precise application-level matching
- **Schema-driven Topics:** Structured topic generation based on configurable data schemas

### Historical Replay
- **Batch Retrieval:** Access historical notifications with configurable batch sizes
- **Sequence-based Access:** Retrieve notifications from specific sequence numbers or timestamps
- **Controlled Historical Notification Delivery:** Controlled replay to prevent system overload

### Robust Architecture
- **Well Abstracted Storage Backend:** Reliable message persistence using configurable backends
- **Schema Validation:** Comprehensive field validation with support for dates, times, integers, and enums
- **Graceful Shutdown:** Clean resource cleanup and connection handling
- **Structured Logging:** Comprehensive observability with configurable log formats

## Use Cases
- **Data Availability Monitoring:** Track when new datasets become available
- **Operational Workflows:** Trigger downstream processes based on data events
- **System Integration:** Connect disparate systems through standardized event notifications
- **Audit and Compliance:** Historical replay for tracking data processing workflows