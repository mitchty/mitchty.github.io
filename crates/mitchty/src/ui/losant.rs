use bevy::prelude::*;
use std::collections::VecDeque;

#[cfg(target_arch = "wasm32")]
use std::sync::{Arc, Mutex};

#[cfg(not(target_arch = "wasm32"))]
use bevy::tasks::{AsyncComputeTaskPool, Task};

/// The `data` payload embedded in each device state stream event.
///
/// Maps to the inner `"data"` object of the stateStream json messages:
/// ```json
/// {"time":"...","data":{"roll":-179.41,"pitch":-58.73},"relayId":"...","relayType":"device","method":"mqtt"}
/// ```
///
/// Only `roll` and `pitch` are needed; all other fields are ignored cause I
/// don't need em really.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DeviceData {
    /// Roll angle in degrees from the device. [-180, 180]
    pub roll: f32,
    /// Pitch angle in degrees from the device. [-180, 180]
    pub pitch: f32,
}

/// Top-level struct for a device state stream event.
///
/// Only `time` and `data` are deserialized everything else is ignored cause I
/// don't need it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DeviceStream {
    /// ISO-8601 timestamp string from the api, this I might use later but not just yet.
    #[allow(dead_code)]
    pub time: String,
    /// Orientation payload is hard coded for now.
    pub data: DeviceData,
}

/// Bevy message emitted each frame for every decoded `DeviceStream` event
/// received over the SSE state stream.
///
/// Carries only the `DeviceData` roll and pitch data struct.
#[derive(Debug, Clone)]
pub struct DeviceStateEvent(pub DeviceData);

impl bevy::ecs::message::Message for DeviceStateEvent {}

/// Try to parse a raw SSE payload into a `DeviceStream` struct.
///
/// Returns `None` and logs a debug warning on any parse failures.
pub fn parse_device_stream(payload: &str) -> Option<DeviceStream> {
    match serde_json::from_str::<DeviceStream>(payload) {
        Ok(ds) => Some(ds),
        Err(e) => {
            bevy::log::debug!(
                "parse_device_stream: failed to parse api payload: {e} was: {payload:?}"
            );
            None
        }
    }
}

// TODO: This is probably a good option for a lib/crate.

/// Enum of a Losant GitHub authentication flow.
// TODO: I cheesed everything here and only care about gh auth cause thats what
// I registered with. Should be more general.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum LosantAuthStatus {
    /// Not authed at all or auth was reset
    #[default]
    Idle,
    /// An auth request is being attempted.
    InFlight,
    /// Auth succeeded and bearer token is in `LosantState::bearer_token`.
    Success,
    /// Auth failed, the inner string is a human-readable error message for
    /// gooey abusage.
    Error(String),
}

/// Application visible to the authenticated user.
#[derive(Debug, Clone)]
pub struct LosantApplication {
    pub id: String,
    pub name: String,
}

/// A user visible device within an application.
#[derive(Debug, Clone)]
pub struct LosantDevice {
    pub id: String,
    pub name: String,
}

/// Enum for app discovery status and device enumeration.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum LosantDiscoveryStatus {
    /// Not fetched anything.
    #[default]
    Idle,
    /// Fetching application list.
    FetchingApps,
    /// Fetching devices for an application.
    FetchingDevices,
    /// Discovery completed.
    Ready,
    /// Discovery failed and like the auth enum the inner string is a
    /// human-readable error message for gooey abusage.
    Error(String),
}

/// Carries the result of a discovery fetch api query back to the main thread.
#[derive(Debug)]
pub enum DiscoveryResult {
    Applications(Vec<LosantApplication>),
    Devices(Vec<LosantDevice>),
}

/// Enum for the status of the SSE device connection.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum LosantSseStatus {
    /// The SSE stream is not connected at all or dropped.
    #[default]
    Disconnected,
    /// Connecting to the device SSE api.
    Connecting,
    /// Connected to the SSE device api successfully.
    Connected,
    /// Connection failed and like the other enums the inner string is also a
    /// human-readable error message for gooey abusage.
    Error(String),
}

/// All gooey window related data needed for state.
#[derive(Resource)]
pub struct LosantState {
    /// Github token data from the user.
    pub github_token_input: String,
    /// Status enum of the auth login.
    pub auth_status: LosantAuthStatus,
    /// Bearer token returned after a successful api auth request.
    pub bearer_token: Option<String>,

    /// Status enum of App/Device discovery.
    pub discovery_status: LosantDiscoveryStatus,
    /// List of applications visible to authed user.
    pub applications: Vec<LosantApplication>,
    /// Currently selected application index into `applications` vector.
    pub selected_application: Option<usize>,
    /// List of devices in the selected application.
    pub devices: Vec<LosantDevice>,
    /// Currently selected device index into `devices` vector.
    pub selected_device: Option<usize>,

    /// Status enum of the http sse stream connection.
    pub sse_status: LosantSseStatus,
    /// Ring-buffer of device payloads with newest at front and is capped to max
    /// of 100 for now. Not sure more is useful even 100 is amongus sus and
    /// chosen out of my butt.
    pub event_log: VecDeque<String>,
}

impl Default for LosantState {
    fn default() -> Self {
        Self {
            github_token_input: String::new(),
            auth_status: LosantAuthStatus::Idle,
            bearer_token: None,
            discovery_status: LosantDiscoveryStatus::Idle,
            applications: Vec::new(),
            selected_application: None,
            devices: Vec::new(),
            selected_device: None,
            sse_status: LosantSseStatus::Disconnected,
            event_log: VecDeque::new(),
        }
    }
}

impl LosantState {
    /// Maximum number of events to keep in `event_log`. Didn't really know what
    /// to pick 100's probably too big tbh.
    pub const EVENT_LIMIT: usize = 100;

    /// Push an event and be sure we never go over 100 in size. This could
    /// probably just be a vec with an index but no need to overengineer for
    /// memory usage.
    pub fn push_event(&mut self, event: String) {
        self.event_log.push_front(event);
        self.event_log.truncate(Self::EVENT_LIMIT);
    }
}

/// Store the async task doing the work for doing Auth http POST reqs to the
/// losant api
///
/// Note native just uses a Bevy `Task` directly used by `poll_losant_auth_task`.
///
/// wasm, as is becoming tradition wraps the `wasm_bindgen_futures::spawn_local`
/// which cause nothing in wasm is easy nor sane has no handle we can use like a
/// bevy task so we wrap the thing up so it can be polled each frame similarly
/// to the bevy `Task`
///
/// This hacks used for all the http api shenanigans.
#[derive(Resource, Default)]
pub struct LosantAuthTask(
    #[cfg(not(target_arch = "wasm32"))] pub Option<Task<Result<String, String>>>,
    #[cfg(target_arch = "wasm32")] pub Option<Arc<Mutex<Option<Result<String, String>>>>>,
);

/// Store the async task doing the work for app/device discovery.
#[derive(Resource, Default)]
pub struct LosantDiscoveryTask(
    #[cfg(not(target_arch = "wasm32"))] pub Option<Task<Result<DiscoveryResult, String>>>,
    #[cfg(target_arch = "wasm32")] pub Option<Arc<Mutex<Option<Result<DiscoveryResult, String>>>>>,
);

/// Store the async task doing the work for SSE streaming from the api.
///
/// For wasm, an `AbortController` is `.abort()` ed to cancel the in flight
/// thread so that the loop used by `connect_sse_wasm` is torn down correctly
/// otherwise behaves similar to the other `Resource` es.
#[derive(Resource, Default)]
pub struct LosantSseTask(
    #[cfg(not(target_arch = "wasm32"))] pub Option<Task<Result<(), String>>>,
    #[cfg(target_arch = "wasm32")] pub Option<web_sys::AbortController>,
);

/// Queue type struct that drains on each ECS tick for incoming SSE events.
///
/// Native is just a stupid simple `mpsc` channel wrapped by a `Mutex` so we
/// honor bevy Sync requirements.
///
/// And then wasm, is like the `Resource` setups that `poll_losant_sse` uses
/// similarly.
#[derive(Resource, Default)]
pub struct LosantSseChannel(
    #[cfg(not(target_arch = "wasm32"))]
    pub  Option<std::sync::Mutex<std::sync::mpsc::Receiver<String>>>,
    #[cfg(target_arch = "wasm32")] pub Option<Arc<Mutex<VecDeque<String>>>>,
);

// These async functions just differ in how reqwest is used, native is using
// rustls so I can keep building static binaries. wasm uses the browser fetch
// api underneath.

// Native is just a one shot tokio runtime.
// Wasm used the browser fetch api underneath.

/// POST `{"accessToken": "<github_token>"}` to the Losant GitHub auth endpoint.
///
/// Returns the Losant bearer token on success, or an error message on failure.
///
/// Endpoint: `POST https://api.losant.com/auth/user/github`
///
/// <https://docs.losant.com/rest-api/auth/#authenticate-user-github>
async fn post_github_auth_async(github_token: String) -> Result<String, String> {
    let client = {
        #[cfg(not(target_arch = "wasm32"))]
        {
            reqwest::Client::builder()
                .use_rustls_tls()
                .build()
                .map_err(|e| format!("failed to build http client: {e}"))?
        }
        #[cfg(target_arch = "wasm32")]
        {
            reqwest::Client::new()
        }
    };

    let body = serde_json::json!({ "accessToken": github_token });

    let response = client
        .post("https://api.losant.com/auth/user/github")
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    if !status.is_success() {
        let msg = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["message"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("HTTP {status}: {text}"));
        return Err(msg);
    }

    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Invalid json response: {e}"))?;

    json["token"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Response missing 'token' field: {text}"))
}

/// GET `https://api.losant.com/applications` and return all user visible
/// applications.
///
/// Response schema on success roughly: `{ "items": [{ "applicationId": "blah", "name": "blah" }, blah] }`
///
/// <https://docs.losant.com/rest-api/applications/>
async fn fetch_applications_async(bearer: String) -> Result<DiscoveryResult, String> {
    let client = {
        #[cfg(not(target_arch = "wasm32"))]
        {
            reqwest::Client::builder()
                .use_rustls_tls()
                .build()
                .map_err(|e| format!("Failed to build HTTP client: {e}"))?
        }
        #[cfg(target_arch = "wasm32")]
        {
            reqwest::Client::new()
        }
    };

    let response = client
        .get("https://api.losant.com/applications")
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {bearer}"))
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    if !status.is_success() {
        let msg = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["message"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("http {status}: {text}"));
        return Err(msg);
    }

    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Invalid json: {e}"))?;

    let apps = json["items"]
        .as_array()
        .ok_or_else(|| format!("Response is missing 'items': {text}"))?
        .iter()
        .filter_map(|v| {
            let id = v["applicationId"].as_str()?.to_string();
            let name = v["name"].as_str().unwrap_or(&id).to_string();
            Some(LosantApplication { id, name })
        })
        .collect();

    Ok(DiscoveryResult::Applications(apps))
}

/// GET `https://api.losant.com/applications/{app_id}/devices` and return all
/// visible devices for the application for the authenticated user.
///
/// Response schema on success roughly: `{ "items": [{ "deviceId": "blah", "name": "blah" }, blah] }`
///
/// <https://docs.losant.com/rest-api/devices/>
async fn fetch_devices_async(bearer: String, app_id: String) -> Result<DiscoveryResult, String> {
    let client = {
        #[cfg(not(target_arch = "wasm32"))]
        {
            reqwest::Client::builder()
                .use_rustls_tls()
                .build()
                .map_err(|e| format!("Failed to build HTTP client: {e}"))?
        }
        #[cfg(target_arch = "wasm32")]
        {
            reqwest::Client::new()
        }
    };

    let url = format!("https://api.losant.com/applications/{app_id}/devices");

    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {bearer}"))
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    if !status.is_success() {
        let msg = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["message"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("HTTP {status}: {text}"));
        return Err(msg);
    }

    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Invalid json: {e}"))?;

    let devices = json["items"]
        .as_array()
        .ok_or_else(|| format!("Response missing 'items': {text}"))? // TODO: iff I build a crate this is a builder api candidate.
        .iter()
        .filter_map(|v| {
            let id = v["deviceId"].as_str()?.to_string();
            let name = v["name"].as_str().unwrap_or(&id).to_string();
            Some(LosantDevice { id, name })
        })
        .collect();

    Ok(DiscoveryResult::Devices(devices))
}

/// Drain a completed auth task result into `LosantState`.
pub fn poll_losant_auth_task(mut task_res: ResMut<LosantAuthTask>, mut state: ResMut<LosantState>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(task) = task_res.0.as_mut() else {
            return;
        };

        let Some(result) = bevy::tasks::futures_lite::future::block_on(
            bevy::tasks::futures_lite::future::poll_once(task),
        ) else {
            return;
        };

        task_res.0 = None;
        apply_auth_result(result, &mut state);
    }

    #[cfg(target_arch = "wasm32")]
    {
        let Some(cell) = task_res.0.as_ref() else {
            return;
        };

        let result = {
            let Ok(mut guard) = cell.try_lock() else {
                return;
            };
            guard.take()
        };

        if let Some(result) = result {
            task_res.0 = None;
            apply_auth_result(result, &mut state);
        }
    }
}

/// Helper function to apply the authentication result into `LosantState`
fn apply_auth_result(result: Result<String, String>, state: &mut LosantState) {
    match result {
        Ok(token) => {
            state.bearer_token = Some(token);
            state.auth_status = LosantAuthStatus::Success;
        }
        Err(msg) => {
            state.auth_status = LosantAuthStatus::Error(msg);
        }
    }
}

/// Run the github auth task in the background, nop if an auth is running.
pub fn spawn_losant_auth_task(state: &mut LosantState, task_res: &mut LosantAuthTask) {
    if task_res.0.is_some() {
        return;
    }

    let token = state.github_token_input.trim().to_string();
    state.auth_status = LosantAuthStatus::InFlight;

    #[cfg(not(target_arch = "wasm32"))]
    {
        let task = AsyncComputeTaskPool::get().spawn(async move {
            // I haaaaate the rust async ecosystem and that reqwest uses Tokio
            // and Bevy uses futures-lite so I have to bridge the two stupid
            // things. Async is not my favorite thing in rust.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("Failed to create Tokio runtime: {e}"))?;
            rt.block_on(post_github_auth_async(token))
        });
        task_res.0 = Some(task);
    }

    #[cfg(target_arch = "wasm32")]
    {
        let cell: Arc<Mutex<Option<Result<String, String>>>> = Arc::new(Mutex::new(None));
        task_res.0 = Some(cell.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let result = post_github_auth_async(token).await;
            if let Ok(mut guard) = cell.lock() {
                *guard = Some(result);
            }
        });
    }
}

/// Poll a completed discovery task result into `LosantState`.
pub fn poll_losant_discovery_task(
    mut task_res: ResMut<LosantDiscoveryTask>,
    mut state: ResMut<LosantState>,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(task) = task_res.0.as_mut() else {
            return;
        };

        let Some(result) = bevy::tasks::futures_lite::future::block_on(
            bevy::tasks::futures_lite::future::poll_once(task),
        ) else {
            return;
        };

        task_res.0 = None;
        apply_discovery_result(result, &mut state);
    }

    #[cfg(target_arch = "wasm32")]
    {
        let Some(cell) = task_res.0.as_ref() else {
            return;
        };

        let result = {
            let Ok(mut guard) = cell.try_lock() else {
                return;
            };
            guard.take()
        };

        if let Some(result) = result {
            task_res.0 = None;
            apply_discovery_result(result, &mut state);
        }
    }
}

/// Silly helper function to convert the discovery result data into the state.
/// Probably a From candidate.
fn apply_discovery_result(result: Result<DiscoveryResult, String>, state: &mut LosantState) {
    match result {
        Ok(DiscoveryResult::Applications(apps)) => {
            state.applications = apps;
            state.selected_application = None;
            state.devices.clear();
            state.selected_device = None;
            state.discovery_status = LosantDiscoveryStatus::Ready;
        }
        Ok(DiscoveryResult::Devices(devices)) => {
            state.devices = devices;
            state.selected_device = None;
            state.discovery_status = LosantDiscoveryStatus::Ready;
        }
        Err(msg) => {
            state.discovery_status = LosantDiscoveryStatus::Error(msg);
        }
    }
}

/// Spawn an application-fetch task thread. nop if running.
pub fn spawn_fetch_applications(
    bearer: String,
    state: &mut LosantState,
    task_res: &mut LosantDiscoveryTask,
) {
    if task_res.0.is_some() {
        return;
    }
    state.discovery_status = LosantDiscoveryStatus::FetchingApps;

    #[cfg(not(target_arch = "wasm32"))]
    {
        let task = AsyncComputeTaskPool::get().spawn(async move {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("Failed to build Tokio runtime: {e}"))?;
            rt.block_on(fetch_applications_async(bearer))
        });
        task_res.0 = Some(task);
    }

    #[cfg(target_arch = "wasm32")]
    {
        let cell: Arc<Mutex<Option<Result<DiscoveryResult, String>>>> = Arc::new(Mutex::new(None));
        task_res.0 = Some(cell.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let result = fetch_applications_async(bearer).await;
            if let Ok(mut guard) = cell.lock() {
                *guard = Some(result);
            }
        });
    }
}

/// Spawn a device-fetch task thread. nop if running.
pub fn spawn_fetch_devices(
    bearer: String,
    app_id: String,
    state: &mut LosantState,
    task_res: &mut LosantDiscoveryTask,
) {
    if task_res.0.is_some() {
        return;
    }
    state.discovery_status = LosantDiscoveryStatus::FetchingDevices;

    #[cfg(not(target_arch = "wasm32"))]
    {
        let task = AsyncComputeTaskPool::get().spawn(async move {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("Failed to build Tokio runtime: {e}"))?;
            rt.block_on(fetch_devices_async(bearer, app_id))
        });
        task_res.0 = Some(task);
    }

    #[cfg(target_arch = "wasm32")]
    {
        let cell: Arc<Mutex<Option<Result<DiscoveryResult, String>>>> = Arc::new(Mutex::new(None));
        task_res.0 = Some(cell.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let result = fetch_devices_async(bearer, app_id).await;
            if let Ok(mut guard) = cell.lock() {
                *guard = Some(result);
            }
        });
    }
}

/// native simply opens a connection via `eventsource-client` and forwards each
/// event's data payload over `sender` for later processing.
///
/// Blocks until the stream ends or the receiver is dropped via an intentioinal
/// disconnect signal in the stream.
#[cfg(not(target_arch = "wasm32"))]
fn connect_sse_native(
    bearer: String,
    app_id: String,
    device_id: String,
    sender: std::sync::mpsc::SyncSender<String>,
) -> Result<(), String> {
    use eventsource_client::{self as es, Client as _};
    use futures::StreamExt;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Failed to build Tokio runtime: {e}"))?;

    rt.block_on(async move {
        let url =
            format!("https://api.losant.com/applications/{app_id}/devices/{device_id}/stateStream");

        // TODO: Do I really need this sdk hypertransport crate? Theres enough
        // junk in this app at this point.
        let transport = launchdarkly_sdk_transport::HyperTransport::new_https()
            .map_err(|e| format!("Failed to create HTTPS transport: {e}"))?;

        let client = es::ClientBuilder::for_url(&url)
            .map_err(|e| format!("Invalid SSE URL: {e:?}"))?
            .header("Authorization", &format!("Bearer {bearer}"))
            .map_err(|e| format!("Invalid auth header: {e:?}"))?
            .header("Accept", "text/event-stream")
            .map_err(|e| format!("Invalid accept header: {e:?}"))?
            .build_with_transport(transport);

        let mut stream = client.stream();

        loop {
            match stream.next().await {
                Some(Ok(es::SSE::Event(event))) => {
                    // Channel closed is implicitly disconnect requested via say gooey button clicks.
                    if sender.send(event.data).is_err() {
                        return Ok(());
                    }
                }
                Some(Ok(es::SSE::Comment(_))) => {
                    // Keep-alive comments, which are ignored.
                }
                Some(Ok(es::SSE::Connected(_))) => {
                    // Connection confirmed, basically useless for this function
                }
                Some(Err(e)) => {
                    return Err(format!("SSE stream error: {e:?}"));
                }
                None => {
                    return Ok(());
                }
            }
        }
    })
}

/// Drain the SSE channel and queue each ECS tick and append new events to the log.
/// Also polls the native task for a terminal error and updates `sse_status` if present.
///
/// For each received payload that successfully parses as a `DeviceStream`
/// struct a corresponding `DeviceStateEvent` message is written to the ecs so
/// that `apply_device_state` can orient the loaded 3d scene directly from the
/// live device roll and pitch values at next tick.
pub fn poll_losant_sse(
    mut task_res: ResMut<LosantSseTask>,
    mut channel_res: ResMut<LosantSseChannel>,
    mut state: ResMut<LosantState>,
    mut device_events: bevy::ecs::message::MessageWriter<DeviceStateEvent>,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Drain the mpsc channel - non-blocking.
        let mut disconnected = false;
        if let Some(rx_mutex) = channel_res.0.as_ref()
            && let Ok(rx) = rx_mutex.lock()
        {
            loop {
                match rx.try_recv() {
                    Ok(payload) => {
                        dispatch_sse_payload(payload, &mut state, &mut device_events);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if disconnected {
            channel_res.0 = None;
        }

        // Poll the background task for completion or terminal error.
        if let Some(task) = task_res.0.as_mut()
            && let Some(result) = bevy::tasks::futures_lite::future::block_on(
                bevy::tasks::futures_lite::future::poll_once(task),
            )
        {
            task_res.0 = None;
            channel_res.0 = None;
            match result {
                Ok(()) => {
                    state.sse_status = LosantSseStatus::Disconnected;
                }
                Err(msg) => {
                    state.sse_status = LosantSseStatus::Error(msg);
                }
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        // Drain the uderlying VecDeque that onmessage/onerror callbacks push into.
        // Items are either real json payloads or an error sentinel written by
        // the onerror closure. We have to collect first to avoid holding the
        // mutex across the borrows later.
        let items: Vec<String> = {
            match (*channel_res).0.as_ref() {
                Some(queue) => match queue.try_lock() {
                    Ok(mut guard) => guard.drain(..).collect(),
                    Err(_) => vec![],
                },
                None => vec![],
            }
        };

        for payload in items {
            if let Some(msg) = payload.strip_prefix("__losant_sse_error__:") {
                // connect_sse_wasm hit an error that wasn't an abort so surface
                // the message to the ECS and other components, mostly gooey,
                // then drop the abort controller slot, clear the channel out.
                task_res.0 = None;
                (*channel_res).0 = None;
                state.sse_status = LosantSseStatus::Error(msg.to_string());
                // Nothing more that could be read
                break;
            } else if payload == "__losant_sse_disconnected__" {
                // Remote closed the stream intentionally, cleanup.
                task_res.0 = None;
                (*channel_res).0 = None;
                state.sse_status = LosantSseStatus::Disconnected;
                break;
            }
            dispatch_sse_payload(payload, &mut state, &mut device_events);
        }
    }
}

/// Parse a raw SSE payload and forward it to the event log and underlying Bevy
/// messages for gooey usage.
fn dispatch_sse_payload(
    payload: String,
    state: &mut LosantState,
    device_events: &mut bevy::ecs::message::MessageWriter<DeviceStateEvent>,
) {
    if let Some(ds) = parse_device_stream(&payload) {
        device_events.write(DeviceStateEvent(ds.data));
    }
    state.push_event(payload);
    if state.sse_status == LosantSseStatus::Connecting {
        // First event confirms we're live.
        state.sse_status = LosantSseStatus::Connected;
    }
}

/// Spawn the SSE streaming http connection and wire up underlying event
/// channel/queue. nop if already connected.
pub fn spawn_sse_connect(
    bearer: String,
    app_id: String,
    device_id: String,
    state: &mut LosantState,
    task_res: &mut LosantSseTask,
    channel_res: &mut LosantSseChannel,
) {
    if task_res.0.is_some() {
        return;
    }

    state.sse_status = LosantSseStatus::Connecting;

    #[cfg(not(target_arch = "wasm32"))]
    {
        // Bounded channel back-pressure if the main thread falls behind. With
        // the data I have so far this is not an issue yet but avoiding stack
        // overflows.
        let (tx, rx) = std::sync::mpsc::sync_channel::<String>(256);
        channel_res.0 = Some(std::sync::Mutex::new(rx));

        let task = AsyncComputeTaskPool::get()
            .spawn(async move { connect_sse_native(bearer, app_id, device_id, tx) });
        task_res.0 = Some(task);
    }

    #[cfg(target_arch = "wasm32")]
    {
        let queue: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        (*channel_res).0 = Some(queue.clone());

        // AbortController gives us a cancel handle, pass the signal into the
        // fetch, and controller.abort() to stop the promise.
        let controller = match web_sys::AbortController::new() {
            Ok(c) => c,
            Err(e) => {
                state.sse_status =
                    LosantSseStatus::Error(format!("AbortController creation failed: {e:?}"));
                return;
            }
        };
        let signal = controller.signal();
        task_res.0 = Some(controller);

        wasm_bindgen_futures::spawn_local(async move {
            connect_sse_wasm(bearer, app_id, device_id, queue, signal).await;
        });
    }
}

/// Disconnect the open SSE stream.
///
/// native drops the channel receiver directly the background task notices on the next
/// `sender.send()` and exits clean.
///
/// wasm calls `AbortController::abort()` which rejects the pending
/// `reader.read()` promise with an `AbortError` and causes `connect_sse_wasm`
/// to exit cleanly without pushing an error sentinel string.
pub fn spawn_sse_disconnect(
    state: &mut LosantState,
    task_res: &mut LosantSseTask,
    channel_res: &mut LosantSseChannel,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Dropping the receiver implicitly closes the channel. The underlying
        // task exits on next send.
        channel_res.0 = None;
        task_res.0 = None;
    }

    #[cfg(target_arch = "wasm32")]
    {
        if let Some(controller) = task_res.0.take() {
            controller.abort();
        }
        (*channel_res).0 = None;
    }

    state.sse_status = LosantSseStatus::Disconnected;
}

/// Open an SSE connection using the underlying native browser Fetch API so we
/// can pass: `Authorization: Bearer`
///
/// All the response body is buffered into full lines and then json is
/// deserialized for the reply. So the shape of the response only works with one
/// expected struct. All this stuff runs within
/// `wasm_bindgen_futures::spawn_local` all of the data is drained via
/// `poll_losant_sse`.
///
/// Cancellation is browser-native, the callers pass an `AbortSignal` from an
/// `AbortController` which ends up calling `controller.abort()` that causes the
/// pending `reader.read()` promise to reject with an `AbortError`, which this
/// function detects and exits from cleanly without pushing an error sentinel to
/// the dam gooey.
#[cfg(target_arch = "wasm32")]
async fn connect_sse_wasm(
    bearer: String,
    app_id: String,
    device_id: String,
    queue: Arc<Mutex<VecDeque<String>>>,
    signal: web_sys::AbortSignal,
) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let url =
        format!("https://api.losant.com/applications/{app_id}/devices/{device_id}/stateStream");

    // Build request headers just with a bearer token header.
    let headers = match web_sys::Headers::new() {
        Ok(h) => h,
        Err(e) => {
            push_sentinel(&queue, &format!("Headers::new failed: {e:?}"));
            return;
        }
    };
    let _ = headers.set("Authorization", &format!("Bearer {bearer}"));
    let _ = headers.set("Accept", "text/event-stream");

    let init = web_sys::RequestInit::new();
    init.set_method("GET");
    init.set_headers(&headers);
    init.set_signal(Some(&signal));

    let request = match web_sys::Request::new_with_str_and_init(&url, &init) {
        Ok(r) => r,
        Err(e) => {
            push_sentinel(&queue, &format!("Request creation failed: {e:?}"));
            return;
        }
    };

    let window = match web_sys::window() {
        Some(w) => w,
        None => {
            push_sentinel(&queue, "no window object");
            return;
        }
    };

    let response = match JsFuture::from(window.fetch_with_request(&request)).await {
        Ok(r) => match r.dyn_into::<web_sys::Response>() {
            Ok(resp) => resp,
            Err(e) => {
                push_sentinel(&queue, &format!("fetch response cast failed: {e:?}"));
                return;
            }
        },
        Err(e) => {
            // AbortError means the caller called controller.abort() this isn't
            // gooey material data to surface methinks.
            let is_abort = js_sys::Reflect::get(&e, &"name".into())
                .ok()
                .and_then(|v| v.as_string())
                .map(|s| s == "AbortError")
                .unwrap_or(false);
            if !is_abort {
                push_sentinel(&queue, &format!("fetch failed: {e:?}"));
            }
            return;
        }
    };

    if !response.ok() {
        push_sentinel(&queue, &format!("HTTP {}", response.status()));
        return;
    }

    let body = match response.body() {
        Some(b) => b,
        None => {
            push_sentinel(&queue, "response body was null");
            return;
        }
    };

    let reader: web_sys::ReadableStreamDefaultReader = match body.get_reader().dyn_into() {
        Ok(r) => r,
        Err(e) => {
            push_sentinel(&queue, &format!("get_reader cast failed: {e:?}"));
            return;
        }
    };

    // Read chunk data, accumulate that crap into a line buffer, and parse
    // entire lines into a struct or not.
    let mut remainder = String::new();

    loop {
        let chunk = match JsFuture::from(reader.read()).await {
            Ok(c) => c,
            Err(e) => {
                // AbortError = cancelled intentionally by the user.
                let is_abort = js_sys::Reflect::get(&e, &"name".into())
                    .ok()
                    .and_then(|v| v.as_string())
                    .map(|s| s == "AbortError")
                    .unwrap_or(false);
                if !is_abort {
                    push_sentinel(&queue, &format!("stream read error: {e:?}"));
                }
                break;
            }
        };

        let done = js_sys::Reflect::get(&chunk, &"done".into())
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if done {
            // Server closed the stream on us future me can decide what to do
            // this is a bogus placeholder. I should probably make a struct and
            // state machine setup for this.
            if let Ok(mut g) = queue.lock() {
                g.push_back("__losant_sse_disconnected__".to_string());
            }
            break;
        }

        if let Ok(value) = js_sys::Reflect::get(&chunk, &"value".into()) {
            if let Ok(array) = value.dyn_into::<js_sys::Uint8Array>() {
                remainder.push_str(&String::from_utf8_lossy(&array.to_vec()));
            }
        }

        while let Some(pos) = remainder.find('\n') {
            let line = remainder[..pos].trim_end_matches('\r').to_string();
            remainder = remainder[pos + 1..].to_string();

            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(mut g) = queue.lock() {
                    g.push_back(data.to_string());
                    while g.len() > LosantState::EVENT_LIMIT {
                        g.pop_front();
                    }
                }
            }
            // :keepalive, event:, id:, blank lines, anything that doesn't parse
            // into a struct is ignored effectively.
        }
    }

    let _ = reader.cancel();
}

/// Push an error sentinel into `queue` so `poll_losant_sse` can surface the
/// underlying message the gooey
#[cfg(target_arch = "wasm32")]
fn push_sentinel(queue: &Arc<Mutex<VecDeque<String>>>, msg: &str) {
    if let Ok(mut g) = queue.lock() {
        g.push_back(format!("__losant_sse_error__:{msg}"));
    }
}
