//! Wire protocol between the relay / twin gateway and its clients.
//!
//! Clients: web IDE, desktop, Android, the 3D twin and the Edge Daemon. Transport is a
//! WebSocket with one JSON message per text frame. Every message is
//! `{"v": 1, "type": "<name>", ...}`. Specification: `spec/protocols/relay.md`; JSON
//! Schemas (the source for generated TypeScript types): `spec/schemas/relay/`.
//!
//! All times are integer nanoseconds since the cell epoch. Values are JSON numbers,
//! booleans or strings; the unit is declared in the cell description, never in the value.
//!
//! **Safety.** The hardwired E-STOP is the authority. The only E-STOP message a client can
//! send is [`Body::AdvisoryEstopRequest`]; no message can disable a safety function.
//!
//! ```
//! use synapse_protocol::{parse, Body};
//! let m = parse(r#"{"v":1,"type":"ping","id":7,"t_ns":1000}"#).unwrap();
//! assert!(matches!(m.body, Body::Ping { id: 7, .. }));
//! ```
#![forbid(unsafe_code)]

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;

/// The only protocol version defined so far. Changes inside v1 are additive.
pub const PROTOCOL_VERSION: u32 = 1;

/// Errors of [`parse`].
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// The text is not a valid v1 message (bad JSON, missing `type`, wrong `v`, bad fields).
    #[error("invalid message: {0}")]
    Invalid(#[from] serde_json::Error),
}

/// The `v` field. Serializes as `1`; any other value is rejected on input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProtocolVersion;

impl Serialize for ProtocolVersion {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u32(PROTOCOL_VERSION)
    }
}

impl<'de> Deserialize<'de> for ProtocolVersion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let n = u32::deserialize(d)?;
        if n == PROTOCOL_VERSION {
            Ok(ProtocolVersion)
        } else {
            Err(D::Error::custom(format!(
                "unsupported protocol version {n}, expected {PROTOCOL_VERSION}"
            )))
        }
    }
}

impl JsonSchema for ProtocolVersion {
    fn schema_name() -> Cow<'static, str> {
        "ProtocolVersion".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Protocol version of the message format. Always 1.",
            "type": "integer",
            "const": 1
        })
    }
}

/// Kind of client that opens a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    /// Web IDE in a browser.
    Web,
    /// Desktop application.
    Desktop,
    /// Android application.
    Android,
    /// The 3D digital twin (authority for virtual devices only).
    Twin,
    /// The Edge Daemon next to real hardware.
    Daemon,
}

/// Role granted to a session. A session may hold several.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// No login (judges' virtual lab). Read-only.
    Guest,
    /// Authenticated read-only user.
    Viewer,
    /// Authenticated user that may control simulations and send advisory E-STOP requests.
    Operator,
    /// The twin: publishes virtual-device data and replay streams.
    Twin,
    /// The Edge Daemon: publishes real-device data and deployment status.
    Daemon,
}

impl Role {
    /// Whether a session holding this role may send the given message.
    ///
    /// The relay applies this to every inbound message and answers `forbidden` otherwise.
    ///
    /// ```
    /// use synapse_protocol::{Body, Role};
    /// let b = Body::SimPause { cell: "c".into(), id: None };
    /// assert!(!Role::Guest.may_send(&b));
    /// assert!(Role::Operator.may_send(&b));
    /// ```
    pub fn may_send(self, body: &Body) -> bool {
        use Body::*;
        let common = matches!(
            body,
            Hello { .. } | Ping { .. } | Pong { .. } | Subscribe { .. } | Unsubscribe { .. }
        );
        if common {
            return true;
        }
        match self {
            Role::Guest | Role::Viewer => false,
            Role::Operator => matches!(
                body,
                SimStart { .. }
                    | SimPause { .. }
                    | SimStep { .. }
                    | SimSetInput { .. }
                    | SimCutLink { .. }
                    | SimSpeed { .. }
                    | ReplayPlay { .. }
                    | AdvisoryEstopRequest { .. }
            ),
            Role::Twin => matches!(
                body,
                CellSnapshot { .. }
                    | Telemetry { .. }
                    | Trace { .. }
                    | SimState { .. }
                    | ReplayHeader { .. }
                    | ReplayEvents { .. }
                    | ReplayEnd { .. }
                    | Ack { .. }
                    | Error { .. }
            ),
            Role::Daemon => matches!(
                body,
                Telemetry { .. }
                    | Trace { .. }
                    | DeploymentStatus { .. }
                    | EstopState { .. }
                    | Ack { .. }
                    | Error { .. }
            ),
        }
    }
}

/// Quality of a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    /// Fresh and trustworthy.
    Good,
    /// Older than the tag's expected update period.
    Stale,
    /// Source failed or the value is not trustworthy.
    Bad,
}

/// A value: JSON number, boolean or string. The unit is in the cell description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Value {
    /// Boolean.
    Bool(bool),
    /// Number (integer or floating point).
    Number(serde_json::Number),
    /// Text or enumeration name.
    Text(String),
}

/// Kind of a trace event; the same eight kinds as the reference interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TraceKind {
    /// Output written.
    #[serde(rename = "OUT")]
    Out,
    /// Signal changed.
    #[serde(rename = "SIG")]
    Sig,
    /// Fault raised.
    #[serde(rename = "FAULT")]
    Fault,
    /// Fault reset.
    #[serde(rename = "RESET")]
    Reset,
    /// Event emitted.
    #[serde(rename = "EVENT")]
    Event,
    /// State machine or sequence state entered.
    #[serde(rename = "STATE")]
    State,
    /// Log line.
    #[serde(rename = "LOG")]
    Log,
    /// Motion command.
    #[serde(rename = "MOVE")]
    Move,
}

/// One trace event of one device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TraceEvent {
    /// Device id the event belongs to.
    pub device: String,
    /// Time in nanoseconds since the cell epoch.
    pub t_ns: i64,
    /// Event kind.
    pub kind: TraceKind,
    /// Name of the output, signal, state, fault or event.
    pub name: String,
    /// Rendered value; empty when the kind has none.
    #[serde(default)]
    pub value: String,
}

/// One telemetry sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sample {
    /// Tag id or signal id from the cell description.
    pub id: String,
    /// Time in nanoseconds since the cell epoch.
    pub t_ns: i64,
    /// The value.
    pub value: Value,
    /// Sample quality.
    pub quality: Quality,
}

/// A device in a cell description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceInfo {
    /// Device id, unique in the cell.
    pub id: String,
    /// Device kind (manifest name, for example `generic-mcu` or `generic-plc`).
    pub kind: String,
    /// True when the device runs in the twin; false for real hardware behind a daemon.
    pub simulated: bool,
}

/// Time budget of a link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LinkBudget {
    /// Largest accepted delivery latency, in nanoseconds.
    pub max_latency_ns: i64,
    /// Heartbeat period, in nanoseconds.
    pub heartbeat_ns: i64,
}

/// A link between two devices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LinkInfo {
    /// Link id, unique in the cell.
    pub id: String,
    /// Source device id.
    pub from: String,
    /// Destination device id.
    pub to: String,
    /// Time budget of the link.
    pub budget: LinkBudget,
}

/// A tag (named process value).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TagInfo {
    /// Tag id, used as `Sample.id`.
    pub id: String,
    /// Owning device id.
    pub device: String,
    /// Neuron type name, for example `bool`, `real` or `u16`.
    pub value_type: String,
    /// Unit of the value, for example `degC`; absent for dimensionless values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// True when clients with the operator role may set it with `sim_set_input`.
    pub writable: bool,
}

/// A safety or working zone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZoneInfo {
    /// Zone id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Devices in the zone.
    pub devices: Vec<String>,
}

/// Description of a cell, as produced by Phase D.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CellDescription {
    /// Display name of the cell.
    pub name: String,
    /// Devices.
    pub devices: Vec<DeviceInfo>,
    /// Links with their budgets.
    pub links: Vec<LinkInfo>,
    /// Tags.
    pub tags: Vec<TagInfo>,
    /// Zones.
    pub zones: Vec<ZoneInfo>,
}

/// Header of a replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReplayHeader {
    /// Cell id.
    pub cell: String,
    /// Devices that appear in the events.
    pub devices: Vec<String>,
    /// Time of the first event's reference point, in nanoseconds since the cell epoch.
    pub start_ns: i64,
    /// Version of the replay format (1).
    pub schema_version: u32,
}

/// A replay file: header plus events ordered by time. Same content as the stream messages
/// `replay_header`, `replay_events`, `replay_end`. Written by D6, played by the twin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReplayFile {
    /// Protocol version.
    pub v: ProtocolVersion,
    /// Header.
    pub header: ReplayHeader,
    /// Events in non-decreasing time order.
    pub events: Vec<TraceEvent>,
}

impl ReplayFile {
    /// Checks the invariants: schema version 1, events ordered by time, no event of a
    /// device missing from the header.
    ///
    /// ```
    /// use synapse_protocol::*;
    /// let r = ReplayFile {
    ///     v: ProtocolVersion,
    ///     header: ReplayHeader { cell: "c".into(), devices: vec!["d".into()], start_ns: 0, schema_version: 1 },
    ///     events: vec![],
    /// };
    /// assert!(r.validate().is_ok());
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        if self.header.schema_version != 1 {
            return Err(format!(
                "unsupported replay schema_version {}",
                self.header.schema_version
            ));
        }
        let mut last = i64::MIN;
        for (i, e) in self.events.iter().enumerate() {
            if e.t_ns < last {
                return Err(format!("event {i} is out of time order"));
            }
            last = e.t_ns;
            if !self.header.devices.contains(&e.device) {
                return Err(format!("event {i} names unknown device `{}`", e.device));
            }
        }
        Ok(())
    }
}

/// Status of one device as reported by the Edge Daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceStatus {
    /// Device id.
    pub device: String,
    /// True when the daemon has a live connection to the device.
    pub online: bool,
    /// Identifier of the build running on the device, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
    /// Time of the last heartbeat, in nanoseconds since the cell epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_ns: Option<i64>,
}

/// Channel a client can subscribe to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// Cell description snapshots and simulation state.
    Cell,
    /// Telemetry samples.
    Telemetry,
    /// Trace events.
    Trace,
    /// Deployment status of real devices.
    Deployment,
    /// Replay streams.
    Replay,
    /// E-STOP state reports.
    Safety,
}

/// Error codes carried by the `error` message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The frame is not a valid message.
    BadMessage,
    /// The `v` or `protocol` value is not supported.
    UnsupportedVersion,
    /// The token in `hello` is invalid or expired.
    Unauthenticated,
    /// The session's roles do not allow the message.
    Forbidden,
    /// The cell, device, link or tag does not exist.
    UnknownTarget,
    /// The client sent too much; slow down.
    RateLimited,
    /// The target cannot do it now (twin or daemon not connected, replay running).
    NotReady,
    /// Server fault.
    Internal,
}

/// State of the hardwired E-STOP circuit as reported by the Edge Daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EstopState {
    /// Circuit closed; machine may run.
    Released,
    /// Circuit open; machine is stopped.
    Tripped,
    /// The daemon is offline or has not reported.
    Unknown,
}

/// A protocol message: the version plus a body tagged by `type`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Message {
    /// Protocol version, always 1.
    pub v: ProtocolVersion,
    /// The message itself.
    #[serde(flatten)]
    pub body: Body,
}

impl Message {
    /// Wraps a body in a v1 message.
    pub fn new(body: Body) -> Self {
        Message {
            v: ProtocolVersion,
            body,
        }
    }

    /// Serializes to one JSON text frame.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Parses one JSON text frame into a message.
///
/// Rejects bad JSON, a missing `type`, an unknown `type` and any `v` other than 1.
pub fn parse(text: &str) -> Result<Message, ProtocolError> {
    Ok(serde_json::from_str(text)?)
}

/// The message bodies, tagged by `type`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Body {
    /// Client to server, first message of a session.
    Hello {
        /// Kind of client.
        client: ClientKind,
        /// Highest protocol version the client supports.
        protocol: u32,
        /// Client software version, informational.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_version: Option<String>,
        /// Optional features the client understands.
        #[serde(default)]
        capabilities: Vec<String>,
        /// Bearer token. Absent for guests.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
    },
    /// Server to client, answer to `hello`.
    Welcome {
        /// Session id.
        session: String,
        /// Server time in nanoseconds since the Unix epoch, for a first clock estimate.
        server_time_ns: i64,
        /// Roles granted to the session.
        roles: Vec<Role>,
        /// Negotiated protocol version.
        protocol: u32,
        /// Optional features the server enables.
        #[serde(default)]
        capabilities: Vec<String>,
    },
    /// Server to client. The session stays open unless the code is fatal (see the spec).
    Error {
        /// Error code.
        code: ErrorCode,
        /// Human-readable text, English.
        message: String,
        /// Request id this answers, when there was one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Server to client. Confirms a request that has no other reply.
    Ack {
        /// Request id being confirmed.
        id: String,
        /// The `type` of the confirmed request.
        request: String,
    },
    /// Either direction. Answered with `pong`.
    Ping {
        /// Sender-chosen sequence number, echoed in `pong`.
        id: u64,
        /// Sender clock when sent, nanoseconds.
        t_ns: i64,
    },
    /// Either direction. Answer to `ping`.
    Pong {
        /// Echo of the ping id.
        id: u64,
        /// Echo of the ping `t_ns`.
        t_ping_ns: i64,
        /// Responder clock when it received the ping, nanoseconds.
        t_ns: i64,
    },
    /// Client to server. Start receiving channels for a cell or one device.
    Subscribe {
        /// Cell id.
        cell: String,
        /// Restrict to one device; absent means the whole cell.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
        /// Channels wanted.
        channels: Vec<Channel>,
        /// Request id, echoed in `ack` or `error`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Client to server. Stop receiving a cell or device.
    Unsubscribe {
        /// Cell id.
        cell: String,
        /// Restrict to one device; absent means the whole cell.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
        /// Request id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Server to client. Full description of a cell; sent after subscribing and on change.
    CellSnapshot {
        /// Cell id.
        cell: String,
        /// The description.
        description: CellDescription,
    },
    /// Publisher (twin or daemon) to server to subscribers. Batched samples.
    Telemetry {
        /// Cell id.
        cell: String,
        /// Samples, oldest first.
        samples: Vec<Sample>,
    },
    /// Publisher to server to subscribers. Batched trace events.
    Trace {
        /// Cell id.
        cell: String,
        /// Events, oldest first.
        events: Vec<TraceEvent>,
    },
    /// Operator to twin. Start or resume the simulation of virtual devices.
    SimStart {
        /// Cell id.
        cell: String,
        /// Request id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Operator to twin. Pause the simulation.
    SimPause {
        /// Cell id.
        cell: String,
        /// Request id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Operator to twin. Advance a paused simulation by whole cell steps.
    SimStep {
        /// Cell id.
        cell: String,
        /// Number of fixed steps, at least 1.
        steps: u32,
        /// Request id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Operator to twin. Set an input of a virtual device (a writable tag).
    SimSetInput {
        /// Cell id.
        cell: String,
        /// Device id.
        device: String,
        /// Tag or input name.
        name: String,
        /// New value.
        value: Value,
        /// Request id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Operator to twin. Cut a link at a simulated time.
    SimCutLink {
        /// Cell id.
        cell: String,
        /// Link id.
        link: String,
        /// Simulated time of the cut, nanoseconds since the cell epoch.
        at_ns: i64,
        /// Request id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Operator to twin. Set the speed factor (1.0 is real time).
    SimSpeed {
        /// Cell id.
        cell: String,
        /// Speed factor, greater than 0.
        factor: f64,
        /// Request id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Twin to server to subscribers. Current state of the simulation.
    SimState {
        /// Cell id.
        cell: String,
        /// True while time advances.
        running: bool,
        /// Simulated time, nanoseconds since the cell epoch.
        t_ns: i64,
        /// Current speed factor.
        speed: f64,
    },
    /// Operator to twin. Play a replay file on the twin.
    ReplayPlay {
        /// Cell id.
        cell: String,
        /// The replay to play.
        replay: ReplayFile,
        /// Request id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Twin to subscribers. A replay stream starts.
    ReplayHeader {
        /// The replay header.
        header: ReplayHeader,
    },
    /// Twin to subscribers. Replay events, in time order.
    ReplayEvents {
        /// Cell id.
        cell: String,
        /// Events.
        events: Vec<TraceEvent>,
    },
    /// Twin to subscribers. The replay stream ended.
    ReplayEnd {
        /// Cell id.
        cell: String,
    },
    /// Daemon to server to subscribers. Read-only for clients.
    DeploymentStatus {
        /// Cell id.
        cell: String,
        /// Status of each device the daemon manages.
        devices: Vec<DeviceStatus>,
    },
    /// Client to server. ADVISORY ONLY: asks that the machine be stopped. The hardwired
    /// E-STOP is the authority. This request neither trips nor releases it and cannot be
    /// used to disable any safety function. The answer is `estop_state`.
    AdvisoryEstopRequest {
        /// Cell id.
        cell: String,
        /// Free-text reason, for the log.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// Request id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Daemon to server to subscribers, and the server's answer to an advisory request.
    /// Always the state reported by the daemon, never a state the server decided.
    EstopState {
        /// Cell id.
        cell: String,
        /// Reported state of the hardwired circuit.
        state: EstopState,
        /// Time of the daemon report, nanoseconds since the cell epoch.
        t_ns: i64,
        /// Id of the advisory request this answers, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
}

impl Body {
    /// Every `type` string, in declaration order.
    pub const TYPES: &'static [&'static str] = &[
        "hello",
        "welcome",
        "error",
        "ack",
        "ping",
        "pong",
        "subscribe",
        "unsubscribe",
        "cell_snapshot",
        "telemetry",
        "trace",
        "sim_start",
        "sim_pause",
        "sim_step",
        "sim_set_input",
        "sim_cut_link",
        "sim_speed",
        "sim_state",
        "replay_play",
        "replay_header",
        "replay_events",
        "replay_end",
        "deployment_status",
        "advisory_estop_request",
        "estop_state",
    ];

    /// The `type` string of this body.
    pub fn type_name(&self) -> &'static str {
        match self {
            Body::Hello { .. } => "hello",
            Body::Welcome { .. } => "welcome",
            Body::Error { .. } => "error",
            Body::Ack { .. } => "ack",
            Body::Ping { .. } => "ping",
            Body::Pong { .. } => "pong",
            Body::Subscribe { .. } => "subscribe",
            Body::Unsubscribe { .. } => "unsubscribe",
            Body::CellSnapshot { .. } => "cell_snapshot",
            Body::Telemetry { .. } => "telemetry",
            Body::Trace { .. } => "trace",
            Body::SimStart { .. } => "sim_start",
            Body::SimPause { .. } => "sim_pause",
            Body::SimStep { .. } => "sim_step",
            Body::SimSetInput { .. } => "sim_set_input",
            Body::SimCutLink { .. } => "sim_cut_link",
            Body::SimSpeed { .. } => "sim_speed",
            Body::SimState { .. } => "sim_state",
            Body::ReplayPlay { .. } => "replay_play",
            Body::ReplayHeader { .. } => "replay_header",
            Body::ReplayEvents { .. } => "replay_events",
            Body::ReplayEnd { .. } => "replay_end",
            Body::DeploymentStatus { .. } => "deployment_status",
            Body::AdvisoryEstopRequest { .. } => "advisory_estop_request",
            Body::EstopState { .. } => "estop_state",
        }
    }
}

fn pretty<T: Serialize>(schema: &T) -> String {
    let mut s = serde_json::to_string_pretty(schema).unwrap_or_default();
    s.push('\n');
    s
}

/// JSON Schema of [`Message`] as canonical text. Committed as
/// `spec/schemas/relay/message.schema.json`; a test fails when they differ.
pub fn message_schema_json() -> String {
    pretty(&schemars::schema_for!(Message))
}

/// JSON Schema of [`ReplayFile`] as canonical text. Committed as
/// `spec/schemas/relay/replay.schema.json`; a test fails when they differ.
pub fn replay_schema_json() -> String {
    pretty(&schemars::schema_for!(ReplayFile))
}
