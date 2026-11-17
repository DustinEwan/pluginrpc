//! The stdio-process transport: start a plugin, call it, end it (spec §6, §8, §9).
//!
//! The rest of this crate is medium-neutral: [`crate::write_line`] and [`crate::read_line`] work
//! over any byte stream, and no type above them knows what carries the messages. This module is
//! not medium-neutral, and that is why it is a module rather than the crate root. Three of the
//! five loss reasons (`spawn`, `exited`, `killed`) describe a process the host started, and the
//! manifest's install and build steps (§11) mean nothing unless the plugin is a program on a
//! filesystem. A host that reaches a plugin over a socket reuses the message layer and writes its
//! own transport; it does not inherit these rules.
//!
//! [`Plugin::start`] enforces the lifecycle rule a host otherwise has to remember: the handshake
//! is the first request on the connection, because no other response can be interpreted before
//! the profile is known (§6). The two hosts that drove this design each put that check somewhere
//! different — one in a probe command, one in a connect function — and the one that left the
//! transport free to call first could send a method to a plugin nobody had checked.

use std::collections::BTreeMap;
use std::io::BufWriter;
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::{
    buffered, buffered_writer, parse_response, read_line, write_line, Call, ErrorObject, Handshake,
    Loss, MessageError, MethodName, Request, Response, MIN_HOST_ID, UNCORRELATED_ID,
    UNUSABLE_RESULT,
};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt as _;

/// How long a call waits for the plugin's exit status before it settles for the reason it already
/// has. The reader notices a closed pipe a moment before the exit watcher learns the status, and
/// without this wait a plugin that crashed is reported as having closed its pipe.
const EXIT_STATUS_WAIT: Duration = Duration::from_millis(250);

/// How long [`Plugin::kill`] and [`Drop`] block for the plugin process to be gone, so a host that
/// reports a result never leaves a plugin running behind it.
const KILL_WAIT: Duration = Duration::from_secs(10);

/// The gap between the deadline a plugin is given and the deadline the host keeps, so a plugin
/// that cuts itself off in time still delivers its answer before the host kills it (spec §9). Not
/// configurable: a host that wants to wait longer asks for a longer `host_timeout`.
const DEADLINE_MARGIN: Duration = Duration::from_millis(250);

/// How long the host waits for the two answers the plugin controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timeouts {
    /// How long to wait for the handshake answer, on every connection ([`Plugin::start`]).
    pub handshake_timeout: Duration,
    /// How long to wait for an answer to a request that carries no deadline of its own. A request
    /// that does carry one passes its own wait to [`Plugin::request`] instead.
    pub request_timeout: Duration,
}

impl Default for Timeouts {
    fn default() -> Timeouts {
        Timeouts {
            handshake_timeout: Duration::from_secs(30),
            request_timeout: Duration::from_secs(10),
        }
    }
}

impl Timeouts {
    /// The `deadlineMs` to put in a request whose host-side wait is `host_timeout` (spec §9).
    ///
    /// Use this when the host's configuration names the host's own wait, a scenario's wall clock
    /// for example, and the plugin's budget follows from it. A host whose configuration names the
    /// plugin's budget instead, as a per-invocation `deadlineMs` does, uses
    /// [`Timeouts::host_wait`]. Either way the pair comes from this one rule, and a host that
    /// writes `deadline + margin` or `deadline - margin` at its own call site has written it twice.
    pub fn plugin_deadline(host_timeout: Duration) -> u64 {
        host_timeout
            .saturating_sub(DEADLINE_MARGIN)
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    /// The host-side wait for a request whose `deadlineMs` is `plugin_deadline` (spec §9).
    ///
    /// The inverse of [`Timeouts::plugin_deadline`], for the profile shape where the configured
    /// number IS the plugin's budget: a host configured with a 2000 ms invocation deadline sends
    /// `deadlineMs: 2000` and waits 2250 ms. Deriving that pair the other way round silently
    /// shortens the budget the profile promised its plugin, which is a change to what the host's
    /// own configuration option means.
    pub fn host_wait(plugin_deadline: Duration) -> Duration {
        plugin_deadline + DEADLINE_MARGIN
    }
}

/// How to start one plugin: what to run, and the conditions to run it under.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginConfig {
    /// The plugin's argv: program first, arguments after (spec §11).
    pub command: Vec<String>,
    /// Extra variables for the plugin. The host's own environment is inherited as it is.
    pub env: BTreeMap<String, String>,
    /// The directory to start the plugin in.
    pub cwd: Option<PathBuf>,
    pub timeouts: Timeouts,
}

impl PluginConfig {
    pub fn new(command: impl IntoIterator<Item = impl Into<String>>) -> PluginConfig {
        PluginConfig {
            command: command.into_iter().map(Into::into).collect(),
            env: BTreeMap::new(),
            cwd: None,
            timeouts: Timeouts::default(),
        }
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> PluginConfig {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> PluginConfig {
        self.cwd = Some(cwd.into());
        self
    }

    /// The plugin's program, for messages that name the thing that failed. Empty when `command` is
    /// empty, which is the command [`Plugin::start`] refuses.
    pub fn program_name(&self) -> &str {
        self.command.first().map(String::as_str).unwrap_or_default()
    }
}

/// Why no connection came up. Both cases mean there is no plugin to call, and they are different
/// things to tell an operator: one is a plugin that is not running, the other is a plugin that is
/// running and declined to be a plugin for this host.
#[derive(Debug, Clone, PartialEq)]
pub enum StartError {
    /// The process could not be started, or it gave no readable handshake answer.
    Lost(Loss),
    /// The plugin answered the handshake with an error frame. It is alive, and it refused.
    Refused(ErrorObject),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::Lost(loss) => write!(f, "{loss}"),
            StartError::Refused(error) => write!(
                f,
                "the plugin refused the handshake ({}): {}",
                error.code, error.message
            ),
        }
    }
}

impl std::error::Error for StartError {}

impl From<Loss> for StartError {
    fn from(loss: Loss) -> StartError {
        StartError::Lost(loss)
    }
}

/// One plugin process and the connection to it.
///
/// The handle owns the process: dropping it ends the plugin unless [`Plugin::detach`] said
/// otherwise, because a plugin left running by a dropped handle is invisible to whoever dropped
/// it.
pub struct Plugin {
    config: PluginConfig,
    process_id: u32,
    next_id: AtomicU64,
    stdin: Mutex<BufWriter<ChildStdin>>,
    state: Arc<Mutex<State>>,
    watcher: Mutex<Option<Receiver<()>>>,
    kill: SyncSender<Sender<()>>,
    kill_on_drop: AtomicBool,
    /// The plugin's stderr, one entry per line, so a host can attribute a death to what the plugin
    /// printed. Spec §3 makes the reading of it a rule: a host that captures stderr without
    /// draining it stops the plugin.
    pub stderr_lines: Arc<Mutex<Vec<String>>>,
}

/// The facts the connection keeps together, so a woken waiter finds the reason already recorded.
struct State {
    /// Why the connection ended, once it has. [`Loss::resolve`] decides which reason stands.
    lost: Option<Loss>,
    /// One slot per request still waiting, keyed by the id the host assigned (spec §4).
    slots: BTreeMap<u64, Sender<Response>>,
    /// The most recent refusal whose id the plugin could not determine. The host must not discard
    /// those, because they report a fault on the connection rather than an answer to a request
    /// (spec §4, §5).
    connection_fault: Option<ErrorObject>,
}

impl State {
    /// Record `reason`, keeping the first cause the precedence rules allow (spec §8), and wake
    /// every request still waiting. Dropping the senders makes each waiter's `recv_timeout` return
    /// `Disconnected` immediately, so a plugin that dies while a call is in flight is reported
    /// when it dies and not when the host's wait happens to end.
    fn lose(&mut self, reason: Loss) {
        self.record(reason);
        self.slots.clear();
    }

    /// Record `reason` and wake nobody. Only the reader may discard a request still waiting,
    /// because the reader is the last place a line can be delivered from: a watcher that cleared
    /// the slots would drop an answer already read from the pipe, and §8 makes loss mean that no
    /// answer arrived. The exit watcher records, and the reader's end-of-output wakes the waiters.
    fn record(&mut self, reason: Loss) {
        self.lost = Some(Loss::resolve(self.lost.take(), reason));
    }
}

impl Plugin {
    /// Start the plugin and shake hands with it (spec §6). The handshake is sent here rather than
    /// left to the host, because no other method can be interpreted before the profile is known.
    ///
    /// Whether the declared `protocolVersion` is one this host speaks is the host's rule and not
    /// the mechanism's, so the handshake comes back for the host to check before its first call.
    /// The fields the profile added beside the core four come back on it too, in
    /// [`Handshake::result`], so a host reads them from the answer it already holds instead of
    /// asking the plugin a second time.
    pub fn start(config: &PluginConfig) -> Result<(Plugin, Handshake), StartError> {
        let plugin = Plugin::spawn(config)?;
        let call: Call<Value> = plugin.request(
            HandshakeMethod::Handshake,
            None,
            config.timeouts.handshake_timeout,
        );
        match call {
            Call::Result(result) => {
                // The core four are the mechanism's rule, so the mechanism checks them; the rest
                // of the object is the profile's and travels to the host untouched (spec §5).
                let mut handshake: Handshake =
                    serde_json::from_value(result.clone()).map_err(|e| Loss::Profile {
                        reason: UNUSABLE_RESULT.to_string(),
                        detail: e.to_string(),
                    })?;
                handshake.result = result;
                Ok((plugin, handshake))
            }
            Call::Refusal(error) => Err(StartError::Refused(error)),
            Call::Lost(loss) => Err(StartError::Lost(loss)),
        }
    }

    fn spawn(config: &PluginConfig) -> Result<Plugin, StartError> {
        let Some(program) = config.command.first() else {
            return Err(Loss::Spawn {
                detail: "the plugin's command is empty".to_string(),
            }
            .into());
        };
        let mut command = Command::new(program);
        command
            .args(config.command.iter().skip(1))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .envs(&config.env);
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                return Err(Loss::Spawn {
                    detail: format!("{program}: {e}"),
                }
                .into())
            }
        };
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let process_id = child.id();

        let state = Arc::new(Mutex::new(State {
            lost: None,
            slots: BTreeMap::new(),
            connection_fault: None,
        }));
        let stderr_lines = Arc::new(Mutex::new(Vec::new()));
        let (kill_tx, kill_rx) = std::sync::mpsc::sync_channel(1);
        let (watcher_done_tx, watcher_done_rx) = channel();
        spawn_reader(stdout, Arc::clone(&state));
        spawn_exit_watcher(child, Arc::clone(&state), kill_rx, watcher_done_tx);
        spawn_stderr_drain(stderr, Arc::clone(&stderr_lines));

        Ok(Plugin {
            config: config.clone(),
            process_id,
            next_id: AtomicU64::new(MIN_HOST_ID),
            stdin: Mutex::new(buffered_writer(stdin)),
            state,
            watcher: Mutex::new(Some(watcher_done_rx)),
            kill: kill_tx,
            kill_on_drop: AtomicBool::new(true),
            stderr_lines,
        })
    }

    /// What this plugin was started with.
    pub fn config(&self) -> &PluginConfig {
        &self.config
    }

    pub fn pid(&self) -> u32 {
        self.process_id
    }

    /// Why the connection ended, once it has. `None` means the plugin is still answering.
    pub fn lost(&self) -> Option<Loss> {
        self.state
            .lock()
            .expect("the connection state is never poisoned")
            .lost
            .clone()
    }

    /// The most recent refusal the plugin could not attach to a request (spec §4, §5). A host
    /// reports it as a fault on the connection; it answers no particular call.
    pub fn connection_fault(&self) -> Option<ErrorObject> {
        self.state
            .lock()
            .expect("the connection state is never poisoned")
            .connection_fault
            .clone()
    }

    /// Send one request whose params are the profile's own type, and wait for its answer. This is
    /// [`Plugin::request`] with the params converted, so a profile's client method is usually a
    /// thin wrapper over it.
    pub fn call<T>(
        &self,
        method: impl MethodName,
        params: &impl Serialize,
        timeout: Duration,
    ) -> Call<T>
    where
        T: DeserializeOwned,
    {
        let params = serde_json::to_value(params).expect("request params are serializable");
        self.request(method, Some(params), timeout)
    }

    /// Send one request and wait for the response whose id matches (spec §4).
    ///
    /// The three endings are [`Call`]'s three cases, and only [`Call::Lost`] says anything about
    /// the process. A refusal keeps the plugin: the connection delivered the request and brought
    /// back an answer that says the work did not happen (spec §8).
    pub fn request<T>(
        &self,
        method: impl MethodName,
        params: Option<Value>,
        timeout: Duration,
    ) -> Call<T>
    where
        T: DeserializeOwned,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        // Claim the slot and read the connection's state under one lock. Checking for a loss and
        // then inserting would let a loss land in between, and a slot inserted after `State::lose`
        // cleared the map is never woken: that call would wait out its whole timeout.
        let already_lost = {
            let mut state = self
                .state
                .lock()
                .expect("the connection state is never poisoned");
            match state.lost.clone() {
                Some(loss) => Some(loss),
                None => {
                    state.slots.insert(id, tx);
                    None
                }
            }
        };
        if let Some(loss) = already_lost {
            // The connection is already known to be down. Ask the watcher for the process's own
            // reason before reporting what an observer noticed, so a plugin that died between two
            // calls is reported as having died (spec §8).
            return Call::Lost(self.exit_status(EXIT_STATUS_WAIT).unwrap_or(loss));
        }

        let request = match params {
            Some(params) => Request::new(id, method, params),
            None => Request::empty(id, method),
        };
        if let Err(e) = self.write(&request) {
            // The plugin's stdin is closed, so it is gone or going. An answer it wrote before it
            // went is still in the pipe and the reader may be delivering it now, so the answer is
            // taken when it arrives inside the wait for the process's own reason (§8).
            if let Ok(response) = rx.recv_timeout(EXIT_STATUS_WAIT) {
                return response.into_call();
            }
            self.drop_slot(id);
            return Call::Lost(
                self.exit_status(EXIT_STATUS_WAIT)
                    .unwrap_or_else(|| Loss::Spawn {
                        detail: format!("pipe to {} closed: {e}", self.config.program_name()),
                    }),
            );
        }

        match rx.recv_timeout(timeout) {
            Ok(response) => response.into_call(),
            Err(RecvTimeoutError::Timeout) => {
                self.drop_slot(id);
                Call::Lost(self.exit_status(EXIT_STATUS_WAIT).unwrap_or(Loss::NoAnswer))
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.drop_slot(id);
                Call::Lost(self.exit_status(EXIT_STATUS_WAIT).unwrap_or(Loss::Eof))
            }
        }
    }

    /// End the plugin and wait for it to be gone. The reason is recorded before the kill is
    /// requested, so a call in flight is attributed to the host's kill and not to the exit status
    /// that kill produced (spec §8).
    pub fn kill(&self) {
        self.state
            .lock()
            .expect("the connection state is never poisoned")
            .lose(Loss::Killed);
        let (ack_tx, ack_rx) = channel();
        if self.kill.send(ack_tx).is_ok() {
            let _ = ack_rx.recv_timeout(KILL_WAIT);
        }
    }

    /// Stop killing the plugin when this handle is dropped, because the process has become
    /// someone else's to end.
    pub fn detach(&self) {
        self.kill_on_drop.store(false, Ordering::Relaxed);
    }

    fn write(&self, message: &impl Serialize) -> Result<(), MessageError> {
        let mut stdin = self.stdin.lock().expect("the stdin lock is never poisoned");
        write_line(&mut *stdin, message)
    }

    fn drop_slot(&self, id: u64) {
        self.state
            .lock()
            .expect("the connection state is never poisoned")
            .slots
            .remove(&id);
    }

    /// The reason the connection ended, waiting up to `wait` for a reason that names what happened
    /// to the process to replace one that only names what an observer noticed (spec §8).
    fn exit_status(&self, wait: Duration) -> Option<Loss> {
        let until = Instant::now() + wait;
        loop {
            let recorded = self.lost();
            if matches!(
                recorded,
                Some(Loss::Exited { .. } | Loss::Killed | Loss::Spawn { .. })
            ) {
                return recorded;
            }
            if Instant::now() >= until {
                return recorded;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl std::fmt::Debug for Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plugin")
            .field("program", &self.config.program_name())
            .field("lost", &self.lost())
            .finish_non_exhaustive()
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        if self.kill_on_drop.load(Ordering::Relaxed) {
            self.kill();
        }
        if let Some(rx) = self
            .watcher
            .lock()
            .expect("the watcher lock is never poisoned")
            .take()
        {
            let _ = rx.recv_timeout(KILL_WAIT);
        }
    }
}

/// The one method the mechanism names (spec §1). A profile's vocabulary is its own type
/// implementing [`MethodName`]; this is what the transport sends for itself, and it is public
/// because a host or a suite that asks a plugin to shake hands again — the `process-alive` rule of
/// `plugin-rpc-conformance` does — is asking the mechanism's method and not one of the profile's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeMethod {
    Handshake,
}

impl MethodName for HandshakeMethod {
    fn name(&self) -> &'static str {
        "handshake"
    }

    fn from_wire(name: &str) -> Option<HandshakeMethod> {
        (name == "handshake").then_some(HandshakeMethod::Handshake)
    }
}

/// Read the plugin's stdout until it ends. A line that cannot be decoded is dropped and the
/// connection stays open, so one bad message costs its own request and nothing else (spec §3).
fn spawn_reader(stdout: std::process::ChildStdout, state: Arc<Mutex<State>>) {
    std::thread::spawn(move || {
        let mut reader = buffered(stdout);
        let mut buf = String::new();
        loop {
            match read_line(&mut reader, &mut buf) {
                Ok(Some(line)) => {
                    if let Ok(response) = parse_response(&line) {
                        deliver(response, &state);
                    }
                }
                Ok(None) | Err(_) => {
                    state
                        .lock()
                        .expect("the connection state is never poisoned")
                        .lose(Loss::Eof);
                    return;
                }
            }
        }
    });
}

/// Hand one response to the request waiting on its id. A response whose id matches nothing is
/// discarded, except a refusal carrying the uncorrelated id, which is kept as a connection fault
/// (spec §4).
fn deliver(response: Response, state: &Mutex<State>) {
    let mut state = state
        .lock()
        .expect("the connection state is never poisoned");
    let id = response.id();
    if let Some(slot) = state.slots.remove(&id) {
        // The receiver may have given up at its deadline already. That is a normal consequence of
        // deadlines, and the connection continues.
        let _ = slot.send(response);
        return;
    }
    if id == UNCORRELATED_ID {
        if let Response::Failure(failure) = response {
            state.connection_fault = Some(failure.error);
        }
    }
}

/// Watch the process, and be the only place that ends it. `Child::wait` may be called from one
/// place, so every kill goes through here: the kill request arrives over a channel, and the
/// process's own exit is recorded as [`Loss::Exited`] with its status (spec §8). The sender is
/// dropped when this thread ends, which is how [`Plugin::drop`] learns the process is gone.
/// It records the reason without waking the waiting calls: the reader is still reading, and an
/// answer already in the pipe belongs to the request that asked for it (spec §8).
fn spawn_exit_watcher(
    mut child: Child,
    state: Arc<Mutex<State>>,
    kill_rx: Receiver<Sender<()>>,
    done_tx: Sender<()>,
) {
    std::thread::spawn(move || {
        let status = loop {
            if let Ok(ack) = kill_rx.try_recv() {
                let _ = child.kill();
                let _ = ack.send(());
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(e) => {
                    state
                        .lock()
                        .expect("the connection state is never poisoned")
                        .record(Loss::Spawn {
                            detail: format!("cannot wait for the plugin: {e}"),
                        });
                    drop(done_tx);
                    return;
                }
            }
        };
        state
            .lock()
            .expect("the connection state is never poisoned")
            .record(Loss::Exited {
                code: status.code(),
                signal: exit_signal(&status),
            });
        drop(done_tx);
    });
}

/// The signal that ended the process, where the platform reports one.
#[cfg(unix)]
fn exit_signal(status: &ExitStatus) -> Option<i32> {
    status.signal()
}

#[cfg(not(unix))]
fn exit_signal(_status: &ExitStatus) -> Option<i32> {
    None
}

/// Drain the plugin's stderr into `lines`. Spec §3 makes this a correctness rule and not a
/// courtesy: a pipe that is never read fills, and the plugin blocks writing to it.
fn spawn_stderr_drain(stderr: ChildStderr, lines: Arc<Mutex<Vec<String>>>) {
    std::thread::spawn(move || {
        let mut reader = buffered(stderr);
        let mut buf = String::new();
        while let Ok(Some(line)) = read_line(&mut reader, &mut buf) {
            lines
                .lock()
                .expect("the stderr lock is never poisoned")
                .push(line);
        }
    });
}
