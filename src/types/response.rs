use serde::Serialize;

/// Response structure for successful notification processing
#[derive(Debug, Clone, Serialize)]
pub struct NotificationResponse {
    pub status: String,
    pub request_id: String,
    pub processed_at: String,
}
