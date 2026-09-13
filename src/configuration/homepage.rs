use serde::{Deserialize, Serialize};

/// Public links shown on the homepage. Omitted fields retain their defaults.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct HomepageSettings {
    pub client_documentation_url: String,
    pub client_repository_url: String,
    pub server_documentation_url: String,
    pub server_repository_url: String,
}

impl Default for HomepageSettings {
    fn default() -> Self {
        Self {
            client_documentation_url: "https://sites.ecmwf.int/docs/aviso-client/main/".into(),
            client_repository_url: "https://github.com/ecmwf/aviso-client".into(),
            server_documentation_url: "https://sites.ecmwf.int/docs/aviso-server/main/".into(),
            server_repository_url: "https://github.com/ecmwf/aviso-server".into(),
        }
    }
}

impl HomepageSettings {
    /// Require absolute HTTP(S) links without embedded credentials.
    pub fn validate(&self) -> Result<(), std::io::Error> {
        for (field, value) in [
            ("client_documentation_url", &self.client_documentation_url),
            ("client_repository_url", &self.client_repository_url),
            ("server_documentation_url", &self.server_documentation_url),
            ("server_repository_url", &self.server_repository_url),
        ] {
            // https://example.org/docs is valid; javascript:alert(1) is not.
            let valid = reqwest::Url::parse(value).is_ok_and(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.has_host()
                    && url.username().is_empty()
                    && url.password().is_none()
            });
            if !valid {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "application.homepage.{field} must be an absolute HTTP(S) URL with a host and no embedded credentials"
                    ),
                ));
            }
        }
        Ok(())
    }
}
