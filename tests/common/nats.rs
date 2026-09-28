// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Opt-in for the tests that need a NATS server with JetStream.

/// Whether the JetStream tests run: `AVISO_RUN_NATS_TESTS` is `1` or `true`.
pub fn nats_tests_enabled() -> bool {
    std::env::var("AVISO_RUN_NATS_TESTS")
        .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

/// The NATS server for the JetStream tests: `NATS_URL`, or a local server.
pub fn nats_url() -> String {
    std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into())
}
