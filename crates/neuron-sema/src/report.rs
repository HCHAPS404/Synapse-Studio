//! Analysis reports: timing, resources, memory and verification results.

/// Timing of one activity (task, interrupt, state machine or safety rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityTiming {
    /// Module the activity belongs to.
    pub module: String,
    /// Name of the activity.
    pub name: String,
    /// `task`, `interrupt`, `fsm` or `safety`.
    pub kind: &'static str,
    /// Effective priority (interrupts and safety rules rank above tasks).
    pub priority: i64,
    /// Core assignment.
    pub core: u32,
    /// Release period in nanoseconds, when periodic.
    pub period_ns: Option<u64>,
    /// Deadline in nanoseconds (the period when not given).
    pub deadline_ns: Option<u64>,
    /// Worst-case execution time in nanoseconds, when a timing model is available.
    pub wcet_ns: Option<u64>,
    /// Worst-case blocking by lower-priority critical sections, in nanoseconds.
    pub blocking_ns: u64,
    /// Worst-case response time in nanoseconds, when it could be computed.
    pub response_ns: Option<u64>,
    /// `Some(false)` when the activity can miss its deadline.
    pub schedulable: Option<bool>,
}

/// A shared resource and the activities that use it (spec 06 §6.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceInfo {
    /// Module of the resource.
    pub module: String,
    /// Name of the resource.
    pub name: String,
    /// `var`, `io` or `plc variable`.
    pub kind: &'static str,
    /// Names of the accessing activities.
    pub accessors: Vec<String>,
    /// Highest priority among the accessors (the SRP ceiling).
    pub ceiling: i64,
    /// More than one activity accesses it.
    pub shared: bool,
    /// The accessors run on different cores.
    pub cross_core: bool,
}

/// Memory budget of one module (spec 05 §5.1, property MEM-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryReport {
    /// Module name.
    pub module: String,
    /// Bytes of static data.
    pub static_bytes: u64,
    /// Bytes of stack in the worst preemption chain.
    pub stack_bytes: u64,
    /// Bytes reserved for the runtime.
    pub runtime_bytes: u64,
    /// Sum of the three.
    pub total_bytes: u64,
    /// RAM of the target, when known.
    pub ram_bytes: Option<u64>,
    /// Named parts of the static data and the stacks, for diagnostics.
    pub breakdown: Vec<(String, u64)>,
}

/// One step of a counterexample trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceStep {
    /// Tick number (0 is the initial state).
    pub tick: u64,
    /// Time of the tick in nanoseconds.
    pub time_ns: u64,
    /// State of every automaton, as `machine.state`.
    pub states: Vec<String>,
    /// Truth value of every input at this tick, as `(expression, value)`.
    pub inputs: Vec<(String, bool)>,
}

/// A counterexample to a `verify` property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Counterexample {
    /// Module name.
    pub module: String,
    /// The violated property as written.
    pub property: String,
    /// Steps from the initial state to the violation.
    pub trace: Vec<TraceStep>,
}

/// Everything the analysis computes besides diagnostics.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reports {
    /// Timing of every activity.
    pub timing: Vec<ActivityTiming>,
    /// Shared resources.
    pub resources: Vec<ResourceInfo>,
    /// Memory budget per module.
    pub memory: Vec<MemoryReport>,
    /// Counterexamples of violated properties.
    pub counterexamples: Vec<Counterexample>,
}
