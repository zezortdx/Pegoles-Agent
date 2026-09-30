//! Pegoles Local: a vision-language model on this Mac as the planner.
//!
//! ```text
//! runner ─▶ LocalPlanner ─▶ prompt::build ─▶ InferenceBackend (MLX worker)
//!              ▲                                   │ text
//!              └──── parse::parse (strict) ◀───────┘
//!                         │ typed ComputerAction
//!                         ▼
//!              PlannedCall [act, settle, observe] ─▶ Core executor ─▶ policy
//! ```
//!
//! One action per step: the model sees the objective, a bounded window
//! of recent steps with their results, and the current screen; it
//! proposes one tool call; Pegoles executes it through the same policy
//! path as any provider and observes again. Loop and parse-failure
//! brakes are deterministic and live here, outside the model.

pub mod parse;
pub mod prompt;

use std::collections::hash_map::DefaultHasher;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pegoles_core::CancellationToken;
use pegoles_inference::{
    GenerateRequest, ImageInput, InferenceBackend, InferenceError, LoadReport, ModelFamily,
    VerifiedModel,
};
use pegoles_protocol::ComputerAction;

use crate::planner::{
    CallOutcome, CallOutput, PlannedCall, Planner, PlannerError, PlannerTurn, Screenshot, Step,
};
use parse::{ModelAction, ParsedStep};
use prompt::{HistoryStep, Layout};

/// The loaded model outlives any one task: the app keeps one backend.
pub type SharedBackend = Arc<Mutex<Box<dyn InferenceBackend>>>;

const MAX_RESULT_CHARS: usize = 240;
const RETRY_TEMPERATURE_STEP: f32 = 0.4;
/// Steps remembered for loop detection.
const SEEN_WINDOW: usize = 12;
const OBSERVE_RETRY_MS: u32 = 1_000;
const MAX_TRACE_OUTPUT_CHARS: usize = 1_500;
/// Vision patch (16) x spatial merge (2) for the Qwen3-VL backbone.
const IMAGE_FACTOR: u32 = 32;

#[derive(Clone, Debug)]
pub struct LocalConfig {
    pub model: VerifiedModel,
    /// Longest side of the image the model sees (source is never
    /// upscaled; both sides are rounded to the vision patch grid).
    pub observe_long_side: u32,
    /// Past steps shown to the model (older ones are counted only).
    pub history_steps: usize,
    /// Past screenshots shown to the model (0 = current screen only).
    pub history_images: usize,
    pub max_tokens: u32,
    pub step_timeout: Duration,
    /// Pause after an action before observing (UI settles).
    pub settle_ms: u32,
    /// Extra attempts when a reply cannot be parsed.
    pub max_parse_retries: u32,
    /// Identical actions with no visible change before giving up.
    pub max_stuck_repeats: u32,
}

impl LocalConfig {
    pub fn for_model(model: VerifiedModel) -> Self {
        let thinking = model.spec.family == ModelFamily::MaiUi;
        Self {
            model,
            observe_long_side: 1440,
            history_steps: 6,
            history_images: 0,
            max_tokens: if thinking { 400 } else { 256 },
            step_timeout: Duration::from_secs(120),
            settle_ms: 400,
            max_parse_retries: 2,
            max_stuck_repeats: 3,
        }
    }
}

/// Per-inference record for benchmarks and diagnostics (bounded).
#[derive(Clone, Debug, serde::Serialize)]
pub struct StepTrace {
    pub turn: u32,
    pub attempt: u32,
    pub prompt_tokens: u32,
    pub generation_tokens: u32,
    pub image_size: (u32, u32),
    pub first_token_ms: f64,
    pub generate_ms: f64,
    pub wall_ms: f64,
    pub mlx_peak_bytes: u64,
    pub process_footprint_bytes: Option<u64>,
    pub output: String,
    /// Action verb, or the parse error.
    pub parsed: Result<String, String>,
}

struct Pending {
    step: HistoryStep,
    image: Arc<Vec<u8>>,
}

pub struct LocalPlanner {
    cfg: LocalConfig,
    backend: SharedBackend,
    objective: String,
    screen: Option<Screenshot>,
    screen_png: Arc<Vec<u8>>,
    screen_hash: u64,
    history: VecDeque<(HistoryStep, Arc<Vec<u8>>)>,
    total_steps: usize,
    pending: Option<Pending>,
    need_observe: bool,
    observe_failed: bool,
    cursor: Option<(f64, f64)>,
    /// Recent (action, screen-before) pairs: proposing an action already
    /// taken in the same visual state is a loop, including A-B-A-B
    /// oscillations where the screen does change.
    seen: VecDeque<(String, u64)>,
    repeats: u32,
    turn: u32,
    pub traces: Vec<StepTrace>,
    pub load: Option<LoadReport>,
    /// The host's note that the computer has internet, if it does.
    internet_note: Option<String>,
}

impl LocalPlanner {
    pub fn new(cfg: LocalConfig, backend: SharedBackend) -> Self {
        Self {
            cfg,
            backend,
            objective: String::new(),
            internet_note: None,
            screen: None,
            screen_png: Arc::new(Vec::new()),
            screen_hash: 0,
            history: VecDeque::new(),
            total_steps: 0,
            pending: None,
            need_observe: false,
            observe_failed: false,
            cursor: None,
            seen: VecDeque::new(),
            repeats: 0,
            turn: 0,
            traces: Vec::new(),
            load: None,
        }
    }

    pub fn config(&self) -> &LocalConfig {
        &self.cfg
    }

    fn family(&self) -> ModelFamily {
        self.cfg.model.spec.family
    }

    fn set_screen(&mut self, shot: Screenshot) {
        let mut h = DefaultHasher::new();
        shot.png.hash(&mut h);
        self.screen_hash = h.finish();
        self.screen_png = Arc::new(shot.png.clone());
        self.screen = Some(shot);
    }

    fn ingest(&mut self, outcomes: Vec<CallOutcome>) {
        let Some(pending) = self.pending.take() else {
            // An observe-only call.
            if let Some(Ok(CallOutput::Image(shot))) = outcomes.into_iter().next().map(|o| o.result)
            {
                self.observe_failed = false;
                self.set_screen(shot);
            } else {
                self.observe_failed = true;
                self.need_observe = true;
            }
            return;
        };
        let mut step = pending.step;
        let mut fresh = None;
        match outcomes.into_iter().next().map(|o| o.result) {
            Some(Ok(CallOutput::Image(shot))) => {
                step.result = "OK".into();
                fresh = Some(shot);
            }
            Some(Ok(CallOutput::Text(_))) => step.result = "OK".into(),
            Some(Err(e)) => step.result = clip(&e, MAX_RESULT_CHARS),
            None => step.result = "Not executed.".into(),
        }
        self.history.push_back((step, pending.image));
        self.total_steps += 1;
        while self.history.len() > self.cfg.history_steps {
            self.history.pop_front();
        }
        match fresh {
            Some(shot) => self.set_screen(shot),
            None => self.need_observe = true,
        }
    }

    fn request(&self, hint: Option<&str>, attempt: u32) -> Result<GenerateRequest, PlannerError> {
        let screen = self
            .screen
            .as_ref()
            .ok_or_else(|| PlannerError::Protocol("no observation".into()))?;
        let resize = model_input_size(screen.width, screen.height, self.cfg.observe_long_side);
        let image_from = self.history.len().saturating_sub(self.cfg.history_images);
        let mut images = Vec::new();
        let history: Vec<HistoryStep> = self
            .history
            .iter()
            .enumerate()
            .map(|(i, (step, png))| {
                let mut s = step.clone();
                s.image = i >= image_from && !png.is_empty();
                if s.image {
                    images.push(ImageInput {
                        png: png.clone(),
                        crop: None,
                        resize: Some(resize),
                    });
                }
                s
            })
            .collect();
        images.push(ImageInput {
            png: self.screen_png.clone(),
            crop: None,
            resize: Some(resize),
        });
        let messages = prompt::build(&Layout {
            family: self.family(),
            objective: &self.objective,
            omitted_steps: self.total_steps - self.history.len(),
            history: &history,
            hint,
            internet: self.internet_note.as_deref(),
        });
        Ok(GenerateRequest {
            messages,
            images,
            max_tokens: self.cfg.max_tokens,
            // Greedy first. A retry after an invalid reply samples a little:
            // at temperature 0 the same prompt tends to repeat the mistake.
            temperature: (attempt as f32 * RETRY_TEMPERATURE_STEP).min(0.8),
            timeout: self.cfg.step_timeout,
        })
    }

    /// One inference; a crashed or hung worker is restarted (model
    /// reloaded) once before the error reaches the runner.
    fn infer(
        &mut self,
        req: &GenerateRequest,
        cancel: &CancellationToken,
    ) -> Result<pegoles_inference::GenerateResponse, PlannerError> {
        let cancelled = || cancel.is_cancelled();
        let mut backend = self.backend.lock().unwrap_or_else(|e| e.into_inner());
        let mut attempt = 0;
        loop {
            match backend.ensure_loaded(&self.cfg.model, &cancelled) {
                Ok(Some(report)) => self.load = Some(report),
                Ok(None) => {}
                Err(e) => return Err(map_err(e)),
            }
            match backend.generate(req, &cancelled) {
                Ok(r) => return Ok(r),
                Err(
                    InferenceError::WorkerCrashed(_)
                    | InferenceError::Timeout(_)
                    | InferenceError::Protocol(_),
                ) if attempt == 0 && !cancel.is_cancelled() => attempt += 1,
                Err(e) => return Err(map_err(e)),
            }
        }
    }

    fn turn_for(&mut self, parsed: ParsedStep) -> PlannerTurn {
        let notes: Vec<String> = parsed.thought.iter().cloned().collect();
        match parsed.action {
            ModelAction::Done(summary) => PlannerTurn::Done { notes, summary },
            ModelAction::Fail(reason) => PlannerTurn::Failed { notes, reason },
            ModelAction::Computer(actions) => {
                let key = (parsed.call_json.clone(), self.screen_hash);
                self.repeats = self.seen.iter().filter(|k| **k == key).count() as u32;
                if self.seen.len() == SEEN_WINDOW {
                    self.seen.pop_front();
                }
                self.seen.push_back(key);
                if self.repeats >= self.cfg.max_stuck_repeats {
                    return PlannerTurn::Failed {
                        notes,
                        reason: "Stopped: the local model kept repeating the same action \
                                 on the same screen without getting closer to the goal."
                            .into(),
                    };
                }
                for a in &actions {
                    if let Some(p) = pointer_target(a) {
                        self.cursor = Some(p);
                    }
                }
                self.pending = Some(Pending {
                    step: HistoryStep {
                        thought: parsed.thought.clone(),
                        call_json: parsed.call_json,
                        result: String::new(),
                        image: false,
                    },
                    image: self.screen_png.clone(),
                });
                let mut steps: Vec<Step> = actions.into_iter().map(Step::Act).collect();
                steps.push(Step::Act(ComputerAction::Wait {
                    duration_ms: self.cfg.settle_ms,
                }));
                steps.push(Step::Observe);
                PlannerTurn::Calls {
                    notes,
                    calls: vec![PlannedCall {
                        call_id: format!("local-{}", self.turn),
                        label: parsed.verb,
                        steps: Ok(steps),
                    }],
                }
            }
        }
    }

    fn hint(&self) -> Option<String> {
        (self.repeats >= 1).then(|| {
            "Note: you already tried this action on this same screen and it did not get you \
             closer to the goal. Try a different action or target."
                .to_string()
        })
    }
}

impl Planner for LocalPlanner {
    fn name(&self) -> String {
        format!("pegoles-local:{}", self.cfg.model.spec.id)
    }

    fn set_internet_note(&mut self, note: Option<String>) {
        self.internet_note = note;
    }

    fn start(&mut self, objective: &str, screen: &Screenshot) -> Result<(), PlannerError> {
        self.objective = objective.to_string();
        self.set_screen(screen.clone());
        self.cursor = None;
        Ok(())
    }

    fn next(
        &mut self,
        outcomes: Vec<CallOutcome>,
        cancel: &CancellationToken,
    ) -> Result<PlannerTurn, PlannerError> {
        self.turn += 1;
        if !outcomes.is_empty() || self.pending.is_some() {
            self.ingest(outcomes);
        }
        if self.need_observe {
            self.need_observe = false;
            // After a failed observation give the guest a moment (e.g. the
            // compositor restarting after the model closed something).
            let mut steps = Vec::new();
            if self.observe_failed {
                steps.push(Step::Act(ComputerAction::Wait {
                    duration_ms: OBSERVE_RETRY_MS,
                }));
            }
            steps.push(Step::Observe);
            return Ok(PlannerTurn::Calls {
                notes: vec![],
                calls: vec![PlannedCall {
                    call_id: format!("local-observe-{}", self.turn),
                    label: "screenshot".into(),
                    steps: Ok(steps),
                }],
            });
        }
        let mut hint = self.hint();
        for attempt in 0..=self.cfg.max_parse_retries {
            if cancel.is_cancelled() {
                return Err(PlannerError::Cancelled);
            }
            let req = self.request(hint.as_deref(), attempt)?;
            let res = self.infer(&req, cancel)?;
            let truncated = res.finish == "length" || res.finish == "text_limit";
            let parsed = parse::parse(self.family(), &res.text, truncated, self.cursor);
            self.traces.push(StepTrace {
                turn: self.turn,
                attempt,
                prompt_tokens: res.prompt_tokens,
                generation_tokens: res.generation_tokens,
                image_size: res.image_sizes.last().copied().unwrap_or((0, 0)),
                first_token_ms: res.timings.first_token_ms,
                generate_ms: res.timings.generate_ms,
                wall_ms: res.timings.wall_ms,
                mlx_peak_bytes: res.memory.peak_bytes,
                process_footprint_bytes: res.memory.process_footprint_bytes,
                output: clip(&res.text, MAX_TRACE_OUTPUT_CHARS),
                parsed: parsed
                    .as_ref()
                    .map(|p| p.verb.clone())
                    .map_err(Clone::clone),
            });
            match parsed {
                Ok(step) => return Ok(self.turn_for(step)),
                Err(e) => {
                    hint = Some(format!(
                        "Your previous reply was not a valid action ({e}). Reply again with \
                         exactly one <tool_call>."
                    ));
                }
            }
        }
        Ok(PlannerTurn::Failed {
            notes: vec![],
            reason: format!(
                "Stopped: the local model did not produce a valid action after {} attempts.",
                self.cfg.max_parse_retries + 1
            ),
        })
    }
}

fn map_err(e: InferenceError) -> PlannerError {
    match e {
        InferenceError::Cancelled => PlannerError::Cancelled,
        InferenceError::BadRequest(m) => PlannerError::Protocol(m),
        other => PlannerError::Unavailable(other.to_string()),
    }
}

fn pointer_target(a: &ComputerAction) -> Option<(f64, f64)> {
    match a {
        ComputerAction::MovePointer { x, y }
        | ComputerAction::Click { x, y, .. }
        | ComputerAction::DoubleClick { x, y, .. }
        | ComputerAction::MouseDown { x, y, .. }
        | ComputerAction::MouseUp { x, y, .. }
        | ComputerAction::Scroll { x, y, .. } => Some((*x, *y)),
        ComputerAction::Drag { to_x, to_y, .. } => Some((*to_x, *to_y)),
        _ => None,
    }
}

/// Model input size: aspect preserved, never upscaled, both sides on
/// the vision patch grid.
pub fn model_input_size(width: u32, height: u32, long_side: u32) -> (u32, u32) {
    let (w, h) = (width.max(1) as f64, height.max(1) as f64);
    let scale = (long_side as f64 / w.max(h)).min(1.0);
    let snap = |v: f64| {
        let f = IMAGE_FACTOR as f64;
        (((v * scale) / f).round() * f).max(f) as u32
    };
    (snap(w), snap(h))
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        let mut out: String = s.chars().take(n).collect();
        out.push('…');
        out
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests;
