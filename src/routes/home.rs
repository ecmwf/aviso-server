// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::configuration::{ApplicationSettings, HomepageSettings};
use crate::telemetry::SERVICE_VERSION;
use actix_web::{HttpResponse, Result, web};
use std::fs;
use std::path::PathBuf;

fn render_homepage(html: &str, links: &HomepageSettings) -> String {
    // Scan the template only once: inserted URL text must never become a token.
    // {{SERVER_VERSION}} is a token; a lone { is ordinary template text.
    let replacements = [
        ("SERVER_VERSION", SERVICE_VERSION),
        (
            "CLIENT_DOCUMENTATION_URL",
            links.client_documentation_url.as_str(),
        ),
        (
            "CLIENT_REPOSITORY_URL",
            links.client_repository_url.as_str(),
        ),
        (
            "SERVER_DOCUMENTATION_URL",
            links.server_documentation_url.as_str(),
        ),
        (
            "SERVER_REPOSITORY_URL",
            links.server_repository_url.as_str(),
        ),
    ];
    let mut rendered = String::with_capacity(html.len());
    let mut remaining = html;
    while let Some(start) = remaining.find("{{") {
        rendered.push_str(&remaining[..start]);
        remaining = &remaining[start..];
        let Some(end) = remaining.find("}}") else {
            break;
        };
        let token = &remaining[2..end];
        if let Some((_, value)) = replacements.iter().find(|(key, _)| *key == token) {
            for character in value.chars() {
                match character {
                    '&' => rendered.push_str("&amp;"),
                    '"' => rendered.push_str("&quot;"),
                    '\'' => rendered.push_str("&#39;"),
                    '<' => rendered.push_str("&lt;"),
                    '>' => rendered.push_str("&gt;"),
                    _ => rendered.push(character),
                }
            }
        } else {
            rendered.push_str(&remaining[..end + 2]);
        }
        remaining = &remaining[end + 2..];
    }
    rendered.push_str(remaining);
    rendered
}

#[utoipa::path(
    get,
    path = "/",
    tag = "general",
    responses(
        (status = 200, description = "Homepage HTML content", content_type = "text/html")
    )
)]
pub async fn homepage(settings: web::Data<ApplicationSettings>) -> Result<HttpResponse> {
    let static_files_path = &settings.static_files_path;
    let mut index_path = PathBuf::from(static_files_path);
    index_path.push("index.html");

    let html = fs::read_to_string(index_path).unwrap_or_else(|_| {
        "<h1>Index file not found</h1><p>Please check the static files configuration.</p>"
            .to_string()
    });

    let rendered = render_homepage(&html, &settings.homepage);

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(rendered))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserted_urls_are_not_processed_as_template_tokens() {
        let links = HomepageSettings {
            client_documentation_url: "https://example.org/?q={{SERVER_REPOSITORY_URL}}&x=\"'<>"
                .into(),
            ..Default::default()
        };
        assert_eq!(
            render_homepage("{{CLIENT_DOCUMENTATION_URL}} {{SERVER_VERSION}}", &links),
            format!(
                "https://example.org/?q={{{{SERVER_REPOSITORY_URL}}}}&amp;x=&quot;&#39;&lt;&gt; {SERVICE_VERSION}"
            )
        );
        let html = render_homepage(
            include_str!("../static/index.html"),
            &HomepageSettings::default(),
        );
        assert!(!html.contains("{{"));
        assert!(html.contains(SERVICE_VERSION));
    }
}
