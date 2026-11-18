use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::ProcessError;

type EmitEvent = dyn Fn(&str, Value) -> Result<(), ProcessError> + Send + Sync;

/// Optional desktop event delivery for the shared encoding pipeline.
#[derive(Clone)]
pub struct AppEvents(Arc<EmitEvent>);

impl AppEvents {
    pub fn emit(&self, event: &str, payload: impl Serialize) -> Result<(), ProcessError> {
        (self.0)(event, serde_json::to_value(payload)?)
    }
}

#[cfg(feature = "desktop")]
impl From<tauri::AppHandle> for AppEvents {
    fn from(app: tauri::AppHandle) -> Self {
        use tauri::Emitter;

        Self(Arc::new(move |event, payload| {
            app.emit(event, payload).map_err(ProcessError::from)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn forwards_serialized_events_without_a_desktop_runtime() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let output = received.clone();
        let events = AppEvents(Arc::new(move |name, payload| {
            output.lock().unwrap().push((name.to_string(), payload));
            Ok(())
        }));

        events.emit("transcript-progress", 42).unwrap();
        assert_eq!(
            *received.lock().unwrap(),
            vec![("transcript-progress".to_string(), serde_json::json!(42))]
        );
    }

    #[test]
    fn propagates_delivery_errors() {
        let events = AppEvents(Arc::new(|_, _| {
            Err(ProcessError::Custom("Delivery failed".into()))
        }));
        assert_eq!(
            events.emit("progress", 0).unwrap_err().to_string(),
            "Delivery failed"
        );
    }
}
