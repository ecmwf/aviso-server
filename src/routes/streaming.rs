use crate::notification_backend::replay::StartAt;

pub fn record_start_at_span_fields(start_at: StartAt) {
    match start_at {
        StartAt::Sequence(id) => {
            tracing::Span::current().record("from_id", id);
        }
        StartAt::Date(date) => {
            tracing::Span::current().record("from_date", date.to_rfc3339());
        }
        StartAt::LiveOnly => {}
    }
}
