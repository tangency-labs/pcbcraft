//! Opt-in, window-addressed automation through ordinary egui input and Craft targets.
use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect};
use pcbcraft_chrome::automation::Target;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{PcbApp, ViewId, editor_layout::HostId, editor_windows::viewport};

#[cfg(test)]
mod tests;

const SCHEMA: &str = "pcbcraft-ui/1";

/// Endpoint/Assistant-turn authority, retained after a queue acknowledgement has been sent.
#[derive(Clone, Debug)]
pub(super) struct Owner {
    pub(super) port: u16,
    pub(super) generation: u64,
    pub(super) chat: bool,
    pub(super) stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The ordinary input destination, before a step and again after its own accepted UI pass.
#[derive(Clone, Debug, PartialEq)]
struct Authority {
    target: Option<crate::editor_view::DocumentTarget>,
    workspace: crate::Workspace,
    definition: Option<u64>,
    focused: ViewId,
    visible: Vec<ViewId>,
    revision: u64,
    opening: u64,
    home: bool,
    dialog: std::mem::Discriminant<crate::Dialog>,
}

#[derive(Debug)]
struct Snapshot {
    layout: u64,
    rect: Rect,
    focused: bool,
    modifiers: Modifiers,
    scale: f32,
    targets: Vec<Target>,
    authority: Authority,
}

#[derive(Debug)]
struct Job {
    ticket: u64,
    host: HostId,
    owner: Option<Owner>,
    authority: Authority,
    deadline: Instant,
    frames: VecDeque<Vec<Event>>,
    modifiers: Modifiers,
    issued: bool,
    held: Option<(PointerButton, Pos2)>,
    rect: Rect,
    scale: f32,
    restore: Modifiers,
    guards: Vec<(String, Pos2)>,
}

/// At most one bounded input operation and one completion record per editor session.
#[derive(Debug, Default)]
pub(super) struct Automation {
    pub(super) observing: bool,
    snapshots: BTreeMap<HostId, Snapshot>,
    next_ticket: u64,
    next_layout: u64,
    job: Option<Job>,
    last: Value,
    release: Option<(HostId, Option<PointerButton>, Modifiers)>,
    /// The framework retains raw input over logic-only callbacks. Remove this exact owned
    /// batch before reinjection, preserving any newly appended physical events.
    injected: Option<(egui::ViewportId, Vec<Event>, bool)>,
    /// Input already entered egui on this pass; cancel it after that pass, rather than clearing
    /// a gesture before the same pass can recreate it from its pending button press.
    cancel_after_paint: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    window: u64,
    #[serde(default)]
    prefix: String,
    handle: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Point {
    Named(Named),
    Position(Position),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    handle: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Position {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Button {
    #[default]
    Left,
    Right,
    Middle,
}
impl Button {
    /// Translate the public vocabulary to ordinary egui pointer buttons.
    fn egui(self) -> PointerButton {
        match self {
            Self::Left => PointerButton::Primary,
            Self::Right => PointerButton::Secondary,
            Self::Middle => PointerButton::Middle,
        }
    }
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Keys {
    ctrl: bool,
    shift: bool,
    alt: bool,
    command: bool,
}
impl Keys {
    /// Preserve platform shortcut semantics: Command means Ctrl except on macOS.
    fn egui(self) -> Modifiers {
        let mac = cfg!(target_os = "macos");
        Modifiers {
            ctrl: self.ctrl || (self.command && !mac),
            shift: self.shift,
            alt: self.alt,
            mac_cmd: self.command && mac,
            command: self.command || (self.ctrl && !mac),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Move {
        target: Point,
    },
    Click {
        target: Point,
        #[serde(default)]
        button: Button,
        #[serde(default)]
        modifiers: Keys,
    },
    Drag {
        from: Point,
        to: Point,
        steps: u16,
        #[serde(default)]
        button: Button,
        #[serde(default)]
        modifiers: Keys,
    },
    Key {
        key: String,
        #[serde(default)]
        modifiers: Keys,
    },
    Text {
        text: String,
    },
    Scroll {
        target: Point,
        delta_x: f32,
        delta_y: f32,
        #[serde(default)]
        modifiers: Keys,
    },
}

impl Snapshot {
    /// Resolve only a unique currently visible handle, or finite in-window logical coordinates.
    fn point(&self, point: Point) -> Result<Pos2, String> {
        let point = match point {
            Point::Position(p) => egui::pos2(p.x, p.y),
            Point::Named(name) => {
                let mut matches = self.targets.iter().filter(|target| target.name == name.handle);
                let target = matches.next().ok_or_else(|| format!("No visible UI handle `{}` in this window", name.handle))?;
                if matches.next().is_some() {
                    return Err(format!("UI handle `{}` is ambiguous", name.handle));
                }
                if !target.enabled {
                    return Err(format!("UI handle `{}` is disabled", name.handle));
                }
                target.rect.center()
            }
        };
        if !point.is_finite() || !self.rect.contains(point) {
            return Err("Input coordinates must be finite logical points inside the addressed window".into());
        }
        Ok(point)
    }
}

impl Automation {
    /// Attach the originating endpoint only to a newly accepted job; inspection cannot reassign it.
    pub(super) fn authorize(&mut self, owner: Owner) {
        if let Some(job) = &mut self.job {
            job.owner.get_or_insert(owner);
        }
    }

    /// Another command must not move the input destination between injection and UI handling.
    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Retain a failure and schedule an off-window button release, which cannot activate a control.
    fn cancel(&mut self, reason: &str) {
        self.cancel_after_paint = false;
        if let Some(job) = self.job.take() {
            self.release = Some((job.host, job.held.map(|(button, _)| button), job.restore));
            self.last = json!({"ticket":job.ticket,"state":"cancelled","reason":reason});
        }
    }

    /// Return bounded queue state; clients must wait for completion before another input call.
    fn status(&self) -> Value {
        self.job.as_ref().map_or_else(|| self.last.clone(), |job| json!({"ticket":job.ticket,"state":"pending","remaining_frames":job.frames.len()}))
    }
}

impl PcbApp {
    /// Capture view/opening/revision and modal ownership without mutating presentation or data.
    fn automation_authority(&self, host: HostId) -> Option<Authority> {
        let host = self.views.hosts.get(&host)?;
        let view = self.views.view(host.focused)?;
        Some(Authority {
            target: view.target(),
            workspace: view.workspace,
            definition: view.definition,
            focused: host.focused,
            visible: host.layout.visible_views(),
            revision: self.session.revision(),
            opening: self.session.history_root_revision(),
            home: self.home,
            dialog: std::mem::discriminant(&self.dialog),
        })
    }

    /// Snapshot each native host separately after its accepted UI pass, including definition views.
    /// Logical coordinates belong to this window; another window can publish identical names.
    pub(super) fn automation_painted(&mut self, ctx: &egui::Context) {
        if !self.automation.observing || ctx.will_discard() {
            return;
        }
        let host = self.views.host;
        let released = self.automation.injected.as_ref().is_some_and(|(id, _, releasing)| *id == viewport(host) && *releasing);
        if self.automation.cancel_after_paint && self.automation.job.as_ref().is_some_and(|job| job.host == host) {
            self.automation.cancel("Cancelled by client after the in-flight UI pass");
            self.views.cancel_gestures();
        }
        if self.automation.injected.as_ref().is_some_and(|(id, _, _)| *id == viewport(host)) {
            self.automation.injected = None;
        }
        if released {
            self.automation.release = None;
        }
        let Some(authority) = self.automation_authority(host) else { return };
        let targets = pcbcraft_chrome::automation::targets(ctx);
        let rect = ctx.content_rect();
        let changed = self.automation.snapshots.get(&host).is_none_or(|s| s.rect != rect || s.targets != targets || s.authority != authority);
        if changed {
            self.automation.next_layout = self.automation.next_layout.saturating_add(1);
        }
        let layout = self.automation.snapshots.get(&host).filter(|_| !changed).map_or(self.automation.next_layout, |s| s.layout);
        self.automation.snapshots.insert(
            host,
            Snapshot {
                layout,
                rect,
                focused: ctx.input(|i| i.focused),
                modifiers: ctx.input(|i| i.modifiers),
                scale: ctx.pixels_per_point(),
                targets,
                authority: authority.clone(),
            },
        );
        if let Some(job) = &mut self.automation.job
            && job.host == host
            && job.issued
        {
            job.frames.pop_front();
            job.issued = false;
            job.authority = authority;
            if job.frames.is_empty() {
                self.automation.last = json!({"ticket":job.ticket,"state":"completed","revision":self.session.revision()});
                self.automation.job = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }
        self.automation.snapshots.retain(|host, _| self.views.hosts.contains_key(host));
    }

    /// Dispatch live-only UI commands. Querying opts into target collection on subsequent paints.
    /// Input acknowledges queue acceptance; ui.inspect reports actual accepted-frame completion.
    pub(super) fn automation_command(&mut self, ctx: &egui::Context, id: &str, params: &Value) -> Result<Value, String> {
        self.automation.observing = true;
        if matches!(id, "ui.inspect" | "ui.cancel") {
            if !params.is_null() && params != &json!({}) {
                return Err(format!("{id} takes no parameters"));
            }
            if id == "ui.cancel" {
                if self.automation.job.as_ref().is_some_and(|job| job.issued) {
                    self.automation.cancel_after_paint = true;
                } else {
                    self.automation.cancel("Cancelled by client");
                    self.views.cancel_gestures();
                }
            }
            self.repaint_editors(ctx);
            return Ok(
                json!({"schema":SCHEMA,"operation":self.automation.status(),"coordinates":"window-local logical points","ready_windows":self.automation.snapshots.keys().map(|h|h.0).collect::<Vec<_>>()}),
            );
        }
        if matches!(id, "ui.handles" | "ui.at") {
            let allowed: &[&str] = if id == "ui.at" { &["window", "handle"] } else { &["window", "prefix"] };
            if params.as_object().is_none_or(|object| object.keys().any(|key| !allowed.contains(&key.as_str()))) {
                return Err(format!("{id} takes only {}", allowed.join(", ")));
            }
            let query: Query = serde_json::from_value(params.clone()).map_err(|e| e.to_string())?;
            let host = HostId(query.window);
            if !self.views.hosts.contains_key(&host) {
                return Err("Unknown editor window".into());
            }
            self.repaint_editors(ctx);
            let snapshot = self.automation.snapshots.get(&host).ok_or("UI geometry is not ready; wait for a paint and retry")?;
            if self.automation_authority(host).as_ref() != Some(&snapshot.authority) {
                return Err("UI geometry is stale; wait for a paint and retry".into());
            }
            if id == "ui.at" {
                let point = snapshot.point(Point::Named(Named { handle: query.handle.ok_or("ui.at needs handle")? }))?;
                return Ok(json!({"schema":SCHEMA,"window":host.0,"layout":snapshot.layout,"x":point.x,"y":point.y}));
            }
            if query.handle.is_some() {
                return Err("ui.handles takes prefix, not handle".into());
            }
            let targets = snapshot
                .targets
                .iter()
                .filter(|t| t.name.starts_with(&query.prefix))
                .map(|t| {
                    json!({"name":t.name,"enabled":t.enabled,
                "x":t.rect.center().x,"y":t.rect.center().y,"rect":[t.rect.min.x,t.rect.min.y,t.rect.width(),t.rect.height()]})
                })
                .collect::<Vec<_>>();
            return Ok(json!({"schema":SCHEMA,"window":host.0,"layout":snapshot.layout,"focused":snapshot.focused,"handles":targets}));
        }
        let name = id.strip_prefix("ui.").ok_or("Unknown UI command")?;
        let mut object = params.as_object().ok_or("UI input takes an object")?.clone();
        let host = HostId(object.remove("window").and_then(|v| v.as_u64()).ok_or("UI input needs an integer window ID")?);
        if host != HostId::ROOT {
            return Err("UI input currently supports the original native window (window 0), including split panes; detached windows support named lookup only".into());
        }
        let layout = object.remove("layout").map(|v| v.as_u64().ok_or("layout must be an integer")).transpose()?;
        if object.contains_key("action") {
            return Err("Use the command ID to choose the input action".into());
        }
        object.insert("action".into(), json!(name));
        let action: Action = serde_json::from_value(Value::Object(object)).map_err(|e| e.to_string())?;
        if self.automation.job.is_some() || self.automation.release.is_some() {
            return Err("UI input is busy; wait for ui.inspect or cancel the pending operation".into());
        }
        let snapshot = self.automation.snapshots.get(&host).ok_or("UI geometry is unavailable; call ui.handles and wait for a paint")?;
        if !self.views.hosts.contains_key(&host) {
            return Err("Unknown editor window".into());
        }
        if self.automation_authority(host).as_ref() != Some(&snapshot.authority) || layout.is_some_and(|layout| layout != snapshot.layout) {
            return Err("UI geometry is stale; inspect handles again".into());
        }
        if !snapshot.focused {
            return Err("Focus the addressed editor window before sending UI input".into());
        }
        if self.files.busy() {
            return Err("Wait for the current file operation before sending UI input".into());
        }
        let guards = action.guards(snapshot)?;
        let (frames, modifiers) = action.frames(snapshot)?;
        self.automation.next_ticket = self.automation.next_ticket.checked_add(1).ok_or("UI ticket limit reached")?;
        let ticket = self.automation.next_ticket;
        self.automation.job = Some(Job {
            ticket,
            host,
            owner: None,
            authority: snapshot.authority.clone(),
            deadline: Instant::now() + Duration::from_secs(5),
            frames: frames.into(),
            modifiers,
            issued: false,
            held: None,
            rect: snapshot.rect,
            scale: snapshot.scale,
            restore: snapshot.modifiers,
            guards,
        });
        ctx.request_repaint_of(viewport(host));
        Ok(json!({"schema":SCHEMA,"ticket":ticket,"state":"queued"}))
    }

    /// Inject one bounded frame before egui processes input. Keep it across logic-only callbacks;
    /// only an accepted host paint advances the queue. Physical input always takes precedence.
    pub(super) fn automation_input(&mut self, ctx: &egui::Context, input: &mut RawInput) {
        pcbcraft_chrome::automation::mark_synthetic_input(ctx, false);
        if let Some((id, events, _)) = &self.automation.injected
            && *id == input.viewport_id
        {
            if let Some(start) = input.events.windows(events.len()).position(|window| window == events) {
                input.events.drain(start..start + events.len());
            }
            self.automation.injected = None;
        }
        if let Some(job) = &self.automation.job {
            let owner_valid = job.owner.as_ref().is_some_and(|owner| {
                if owner.stopped.load(std::sync::atomic::Ordering::Acquire) {
                    return false;
                }
                if owner.chat {
                    self.assistant.turn_open() && self.assistant.generation() == Some(owner.generation)
                } else {
                    self.live_control.as_ref().is_some_and(|live| live.port() == owner.port && live.current() == owner.generation)
                }
            });
            let invalid = if !owner_valid {
                Some("Automation connection or Assistant turn ended")
            } else if Instant::now() >= job.deadline {
                Some("UI input timed out before completion")
            } else if self.automation_authority(job.host).as_ref() != Some(&job.authority) {
                Some("Project or input destination changed")
            } else if self.files.busy() {
                Some("A file operation owns the editor")
            } else if input.viewport_id == viewport(job.host)
                && (!input.focused || input.viewport().minimized == Some(true) || input.viewport().occluded == Some(true))
            {
                Some("Editor window lost focus or visibility")
            } else if input.viewport_id == viewport(job.host)
                && (input.screen_rect.is_some_and(|rect| rect != job.rect) || ctx.pixels_per_point() != job.scale)
            {
                Some("Window size or scale changed")
            } else if job.frames.front().is_some_and(|events| events.iter().any(|e| matches!(e, Event::PointerButton { .. })))
                && self.automation.snapshots.get(&job.host).is_none_or(|snapshot| {
                    job.guards.iter().any(|(name, point)| snapshot.point(Point::Named(Named { handle: name.clone() })).ok() != Some(*point))
                })
            {
                Some("Named input target moved, became disabled or was covered")
            } else if input.viewport_id == viewport(job.host) && input.events.iter().any(physical_input) {
                Some("Physical input interrupted automation")
            } else {
                None
            };
            if let Some(reason) = invalid {
                self.automation.cancel(reason);
                self.views.cancel_gestures();
            }
        }
        if let Some((host, button, modifiers)) = self.automation.release
            && input.viewport_id == viewport(host)
        {
            let pos = egui::pos2(-1000.0, -1000.0);
            let mut release = vec![Event::ModifiersChanged(modifiers)];
            if let Some(button) = button {
                release.extend([Event::PointerMoved(pos), Event::PointerButton { pos, button, pressed: false, modifiers }, Event::PointerGone]);
            }
            self.automation.injected = Some((input.viewport_id, release.clone(), true));
            pcbcraft_chrome::automation::mark_synthetic_input(ctx, true);
            release.append(&mut input.events);
            input.events = release;
            ctx.request_repaint_of(viewport(host));
        }
        if let Some(job) = &mut self.automation.job
            && input.viewport_id == viewport(job.host)
            && let Some(events) = job.frames.front()
        {
            let start = input.events.len();
            input.events.push(Event::ModifiersChanged(job.modifiers));
            input.events.extend_from_slice(events);
            self.automation.injected = Some((input.viewport_id, input.events[start..].to_vec(), false));
            pcbcraft_chrome::automation::mark_synthetic_input(ctx, true);
            for event in events {
                if let Event::PointerButton { pos, button, pressed, .. } = event {
                    job.held = pressed.then_some((*button, *pos));
                }
            }
            job.issued = true;
        }
    }
}

/// Events produced by a person cancel a queued synthetic gesture instead of mixing owners.
fn physical_input(event: &Event) -> bool {
    matches!(
        event,
        Event::PointerMoved(_)
            | Event::MouseMoved(_)
            | Event::PointerGone
            | Event::PointerButton { .. }
            | Event::Key { .. }
            | Event::ModifiersChanged(_)
            | Event::Text(_)
            | Event::Paste(_)
            | Event::Ime(_)
            | Event::MouseWheel { .. }
            | Event::Touch { .. }
            | Event::Zoom(_)
            | Event::Rotate(_)
    )
}

impl Action {
    /// Preserve named destinations across hover/layout transitions before a button event.
    fn guards(&self, snapshot: &Snapshot) -> Result<Vec<(String, Pos2)>, String> {
        let points = match self {
            Self::Move { target } | Self::Click { target, .. } | Self::Scroll { target, .. } => vec![target],
            Self::Drag { from, to, .. } => vec![from, to],
            _ => Vec::new(),
        };
        points
            .into_iter()
            .filter_map(|point| match point {
                Point::Named(name) => Some(snapshot.point(Point::Named(Named { handle: name.handle.clone() })).map(|p| (name.handle.clone(), p))),
                _ => None,
            })
            .collect()
    }
    /// Validate the entire action before queuing any events. Drags have at most 120 moves;
    /// text is limited to 4096 bytes and wheel deltas to 10,000 logical points per axis.
    fn frames(self, snapshot: &Snapshot) -> Result<(Vec<Vec<Event>>, Modifiers), String> {
        let mut frames = Vec::new();
        let mut keys = Modifiers::NONE;
        let button = |pos, button, pressed, modifiers| Event::PointerButton { pos, button, pressed, modifiers };
        match self {
            Self::Move { target } => frames.push(vec![Event::PointerMoved(snapshot.point(target)?)]),
            Self::Click { target, button: b, modifiers } => {
                keys = modifiers.egui();
                let pos = snapshot.point(target)?;
                let b = b.egui();
                frames.push(vec![Event::PointerMoved(pos)]);
                frames.push(vec![button(pos, b, true, keys), button(pos, b, false, keys)]);
            }
            Self::Drag { from, to, steps, button: b, modifiers } => {
                if !(2..=120).contains(&steps) {
                    return Err("Drag steps must be between 2 and 120".into());
                }
                keys = modifiers.egui();
                let from = snapshot.point(from)?;
                let to = snapshot.point(to)?;
                let b = b.egui();
                frames.push(vec![Event::PointerMoved(from)]);
                frames.push(vec![button(from, b, true, keys)]);
                for step in 1..=steps {
                    frames.push(vec![Event::PointerMoved(from.lerp(to, f32::from(step) / f32::from(steps)))]);
                }
                frames.push(vec![button(to, b, false, keys)]);
            }
            Self::Key { key, modifiers } => {
                let key = egui::Key::from_name(&key).ok_or("Unknown key; use an egui key name such as W, Escape, Enter or PageDown")?;
                keys = modifiers.egui();
                frames.push(vec![
                    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: keys },
                    Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: keys },
                ]);
            }
            Self::Text { text } => {
                if text.is_empty() || text.len() > 4096 || text.chars().any(|c| c.is_control()) {
                    return Err("Text needs 1–4096 UTF-8 bytes without control characters; use ui.key for Enter or Tab".into());
                }
                frames.push(vec![Event::Text(text)]);
            }
            Self::Scroll { target, delta_x, delta_y, modifiers } => {
                if !delta_x.is_finite() || !delta_y.is_finite() || delta_x.abs() > 10000.0 || delta_y.abs() > 10000.0 {
                    return Err("Wheel deltas must be finite and at most 10000 logical points".into());
                }
                keys = modifiers.egui();
                frames.push(vec![
                    Event::PointerMoved(snapshot.point(target)?),
                    Event::MouseWheel {
                        source: egui::MouseWheelSource::Unknown,
                        phase: egui::TouchPhase::Move,
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(delta_x, delta_y),
                        modifiers: keys,
                    },
                ]);
            }
        }
        if keys != snapshot.modifiers {
            frames.push(vec![Event::ModifiersChanged(snapshot.modifiers)]);
        }
        Ok((frames, keys))
    }
}

/// Discover the live automation contract beside the existing project/window command catalog.
pub(super) fn catalog() -> Vec<Value> {
    let point = json!({"oneOf":[{"type":"object","properties":{"handle":{"type":"string"}},"required":["handle"],"additionalProperties":false},
        {"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"}},"required":["x","y"],"additionalProperties":false}]});
    let keys = json!({"type":"object","properties":{"ctrl":{"type":"boolean"},"shift":{"type":"boolean"},"alt":{"type":"boolean"},"command":{"type":"boolean"}},"additionalProperties":false});
    let button = json!({"type":"string","enum":["left","right","middle"],"default":"left"});
    [
        ("inspect","Inspect queued UI input",json!({}),vec![]),
        ("cancel","Cancel queued UI input",json!({}),vec![]),
        ("handles","List visible named controls in one window",json!({"window":{"type":"integer"},"prefix":{"type":"string"}}),vec!["window"]),
        ("at","Locate one unique enabled control",json!({"window":{"type":"integer"},"handle":{"type":"string"}}),vec!["window","handle"]),
        ("move","Move the pointer through ordinary UI input",json!({"target":point}),vec!["target"]),
        ("click","Click through ordinary UI input",json!({"target":point,"button":button,"modifiers":keys}),vec!["target"]),
        ("drag","Drag over successive UI frames",json!({"from":point,"to":point,"steps":{"type":"integer","minimum":2,"maximum":120},"button":button,"modifiers":keys}),vec!["from","to","steps"]),
        ("key","Press and release a key",json!({"key":{"type":"string"},"modifiers":keys}),vec!["key"]),
        ("text","Type text into the focused field",json!({"text":{"type":"string","minLength":1,"maxLength":4096}}),vec!["text"]),
        ("scroll","Scroll at a named target or logical point",json!({"target":point,"delta_x":{"type":"number"},"delta_y":{"type":"number"},"modifiers":keys}),vec!["target","delta_x","delta_y"]),
    ].into_iter().map(|(name,title,mut properties,mut required)| {
        if !matches!(name,"inspect"|"cancel"|"handles"|"at") { properties["window"]=json!({"type":"integer"});properties["layout"]=json!({"type":"integer"});required.push("window"); }
        json!({"id":format!("ui.{name}"),"title":title,"summary":format!("{title}. Native live editor only. Input returns a ticket; poll ui.inspect until completed before sending more input. Coordinates are window-local logical points."),
            "scope":"window","params":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
    }).collect()
}
