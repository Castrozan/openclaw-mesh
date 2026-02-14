use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, canvas::Canvas},
    Terminal,
};
use serde::Deserialize;
use std::{
    io::stdout,
    process::Command,
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant},
};

const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;
const DEFAULT_TICK_RATE_MS: u64 = 50;
const DEFAULT_ACTIVE_THRESHOLD_MINUTES: u64 = 5;
const DEFAULT_CAMERA_ANGLE_SPEED: f64 = 0.020;
const DEFAULT_CAMERA_PITCH_SPEED: f64 = 0.006;
const DEFAULT_CAMERA_DISTANCE: f64 = 8.0;
const DEFAULT_PULSE_SPEED: f64 = 0.15;
const DEFAULT_PULSE_DECAY: f64 = 0.95;
const DEFAULT_DEPTH_FADE_STRENGTH: f64 = 0.75;
const DEFAULT_EDGE_FADE_STRENGTH: f64 = 0.85;
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 2;
const COLOR_QUANTIZE_STEP: u8 = 8;

fn default_color_local() -> Vec<u8> { vec![50, 255, 50] }
fn default_color_grid() -> Vec<u8> { vec![50, 200, 255] }
fn default_color_edge_active() -> Vec<u8> { vec![0, 255, 255] }
fn default_color_edge_inactive() -> Vec<u8> { vec![0, 220, 240] }
fn default_color_inactive_node() -> Vec<u8> { vec![40, 60, 40] }
fn default_color_name_active() -> Vec<u8> { vec![100, 255, 100] }
fn default_color_name_inactive() -> Vec<u8> { vec![0, 150, 0] }
fn default_color_model_active() -> Vec<u8> { vec![255, 255, 100] }
fn default_color_model_inactive() -> Vec<u8> { vec![150, 150, 50] }
fn default_color_tokens() -> Vec<u8> { vec![150, 220, 255] }
fn default_color_border() -> Vec<u8> { vec![0, 100, 0] }
fn default_color_title() -> Vec<u8> { vec![50, 255, 50] }

fn default_poll_interval_secs() -> u64 { DEFAULT_POLL_INTERVAL_SECS }
fn default_tick_rate_ms() -> u64 { DEFAULT_TICK_RATE_MS }
fn default_active_threshold_minutes() -> u64 { DEFAULT_ACTIVE_THRESHOLD_MINUTES }
fn default_camera_angle_speed() -> f64 { DEFAULT_CAMERA_ANGLE_SPEED }
fn default_camera_pitch_speed() -> f64 { DEFAULT_CAMERA_PITCH_SPEED }
fn default_camera_distance() -> f64 { DEFAULT_CAMERA_DISTANCE }
fn default_pulse_speed() -> f64 { DEFAULT_PULSE_SPEED }
fn default_pulse_decay() -> f64 { DEFAULT_PULSE_DECAY }
fn default_depth_fade_strength() -> f64 { DEFAULT_DEPTH_FADE_STRENGTH }
fn default_edge_fade_strength() -> f64 { DEFAULT_EDGE_FADE_STRENGTH }
fn default_connect_timeout_secs() -> u64 { DEFAULT_CONNECT_TIMEOUT_SECS }

#[derive(Deserialize, Debug, Clone)]
struct MeshConfig {
    #[serde(default)]
    grid: Vec<GridAgentConfig>,
    #[serde(default)]
    connections: ConnectionsConfig,
    #[serde(default)]
    colors: ColorsConfig,
    #[serde(default)]
    motion: MotionConfig,
    #[serde(default)]
    timing: TimingConfig,
}

impl Default for MeshConfig {
    fn default() -> Self {
        Self {
            grid: vec![],
            connections: ConnectionsConfig::default(),
            colors: ColorsConfig::default(),
            motion: MotionConfig::default(),
            timing: TimingConfig::default(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
struct GridAgentConfig {
    id: String,
    #[serde(default)]
    emoji: String,
    #[serde(default)]
    model: String,
}

#[derive(Deserialize, Debug, Clone)]
struct ConnectionsConfig {
    #[serde(default, rename = "sshHost")]
    ssh_host: Option<String>,
    #[serde(default, rename = "sshUser")]
    ssh_user: Option<String>,
    #[serde(default = "default_connect_timeout_secs", rename = "connectTimeoutSecs")]
    connect_timeout_secs: u64,
}

impl Default for ConnectionsConfig {
    fn default() -> Self {
        Self {
            ssh_host: None,
            ssh_user: None,
            connect_timeout_secs: DEFAULT_CONNECT_TIMEOUT_SECS,
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
struct ColorsConfig {
    #[serde(default = "default_color_local")]
    local: Vec<u8>,
    #[serde(default = "default_color_grid")]
    grid: Vec<u8>,
    #[serde(default = "default_color_edge_active", rename = "edgeActive")]
    edge_active: Vec<u8>,
    #[serde(default = "default_color_edge_inactive", rename = "edgeInactive")]
    edge_inactive: Vec<u8>,
    #[serde(default = "default_color_inactive_node", rename = "inactiveNode")]
    inactive_node: Vec<u8>,
    #[serde(default = "default_color_name_active", rename = "nameActive")]
    name_active: Vec<u8>,
    #[serde(default = "default_color_name_inactive", rename = "nameInactive")]
    name_inactive: Vec<u8>,
    #[serde(default = "default_color_model_active", rename = "modelActive")]
    model_active: Vec<u8>,
    #[serde(default = "default_color_model_inactive", rename = "modelInactive")]
    model_inactive: Vec<u8>,
    #[serde(default = "default_color_tokens")]
    tokens: Vec<u8>,
    #[serde(default = "default_color_border")]
    border: Vec<u8>,
    #[serde(default = "default_color_title")]
    title: Vec<u8>,
}

impl Default for ColorsConfig {
    fn default() -> Self {
        Self {
            local: default_color_local(),
            grid: default_color_grid(),
            edge_active: default_color_edge_active(),
            edge_inactive: default_color_edge_inactive(),
            inactive_node: default_color_inactive_node(),
            name_active: default_color_name_active(),
            name_inactive: default_color_name_inactive(),
            model_active: default_color_model_active(),
            model_inactive: default_color_model_inactive(),
            tokens: default_color_tokens(),
            border: default_color_border(),
            title: default_color_title(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
struct MotionConfig {
    #[serde(default = "default_camera_angle_speed", rename = "cameraAngleSpeed")]
    camera_angle_speed: f64,
    #[serde(default = "default_camera_pitch_speed", rename = "cameraPitchSpeed")]
    camera_pitch_speed: f64,
    #[serde(default = "default_camera_distance", rename = "cameraDistance")]
    camera_distance: f64,
    #[serde(default = "default_pulse_speed", rename = "pulseSpeed")]
    pulse_speed: f64,
    #[serde(default = "default_pulse_decay", rename = "pulseDecay")]
    pulse_decay: f64,
    #[serde(default = "default_depth_fade_strength", rename = "depthFadeStrength")]
    depth_fade_strength: f64,
    #[serde(default = "default_edge_fade_strength", rename = "edgeFadeStrength")]
    edge_fade_strength: f64,
}

impl Default for MotionConfig {
    fn default() -> Self {
        Self {
            camera_angle_speed: DEFAULT_CAMERA_ANGLE_SPEED,
            camera_pitch_speed: DEFAULT_CAMERA_PITCH_SPEED,
            camera_distance: DEFAULT_CAMERA_DISTANCE,
            pulse_speed: DEFAULT_PULSE_SPEED,
            pulse_decay: DEFAULT_PULSE_DECAY,
            depth_fade_strength: DEFAULT_DEPTH_FADE_STRENGTH,
            edge_fade_strength: DEFAULT_EDGE_FADE_STRENGTH,
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
struct TimingConfig {
    #[serde(default = "default_poll_interval_secs", rename = "pollIntervalSecs")]
    poll_interval_secs: u64,
    #[serde(default = "default_tick_rate_ms", rename = "tickRateMs")]
    tick_rate_ms: u64,
    #[serde(default = "default_active_threshold_minutes", rename = "activeThresholdMinutes")]
    active_threshold_minutes: u64,
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
            tick_rate_ms: DEFAULT_TICK_RATE_MS,
            active_threshold_minutes: DEFAULT_ACTIVE_THRESHOLD_MINUTES,
        }
    }
}

fn load_mesh_config() -> MeshConfig {
    let config_path = dirs::config_dir()
        .map(|d| d.join("openclaw-mesh/config.json"))
        .unwrap_or_default();

    std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

fn rgb_from_vec(v: &[u8]) -> (u8, u8, u8) {
    (
        v.first().copied().unwrap_or(0),
        v.get(1).copied().unwrap_or(0),
        v.get(2).copied().unwrap_or(0),
    )
}

fn color_from_vec(v: &[u8]) -> Color {
    let (r, g, b) = rgb_from_vec(v);
    Color::Rgb(r, g, b)
}

fn quantize_color(color: Color, step: u8) -> Color {
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r / step) * step,
            (g / step) * step,
            (b / step) * step,
        ),
        other => other,
    }
}

#[derive(Debug, Clone)]
struct AgentNode {
    id: String,
    label: String,
    emoji: String,
    model: String,
    active: bool,
    active_sessions: u32,
    total_sessions: u32,
    total_tokens: u64,
    kind: AgentKind,
    pos3d: [f64; 3],
    screen_x: f64,
    screen_y: f64,
    screen_depth: f64,
    pulse_phase: f64,
    is_local: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum AgentKind {
    Local,
    Grid,
}

impl AgentKind {
    fn base_color(&self, config: &ColorsConfig) -> (u8, u8, u8) {
        match self {
            AgentKind::Local => rgb_from_vec(&config.local),
            AgentKind::Grid => rgb_from_vec(&config.grid),
        }
    }
}

#[derive(Deserialize, Debug)]
struct AgentsListEntry {
    id: String,
    #[serde(rename = "identityName")]
    identity_name: Option<String>,
    #[serde(rename = "identityEmoji")]
    identity_emoji: Option<String>,
    model: Option<String>,
}

#[derive(Clone)]
struct PollResult {
    agents: Vec<AgentNode>,
    gateway_online: bool,
    status_message: String,
}

struct App {
    agents: Vec<AgentNode>,
    edges: Vec<(usize, usize)>,
    tick: u64,
    last_poll: Instant,
    gateway_online: bool,
    status_message: String,
    show_active_only: bool,
    camera_angle: f64,
    camera_pitch: f64,
    angle_speed: f64,
    pitch_speed: f64,
    target_angle_speed: f64,
    target_pitch_speed: f64,
    direction_timer: u64,
    work_online: Arc<AtomicBool>,
    poll_result: Arc<Mutex<Option<PollResult>>>,
    polling: Arc<AtomicBool>,
    config: MeshConfig,
}

impl App {
    fn new(active_only: bool, config: MeshConfig) -> Self {
        let poll_interval = Duration::from_secs(config.timing.poll_interval_secs);
        Self {
            agents: vec![],
            edges: vec![],
            tick: 0,
            last_poll: Instant::now() - poll_interval - Duration::from_secs(1),
            gateway_online: false,
            status_message: String::from("connecting..."),
            show_active_only: active_only,
            camera_angle: 0.0,
            camera_pitch: 0.25,
            angle_speed: config.motion.camera_angle_speed,
            pitch_speed: 0.0,
            target_angle_speed: config.motion.camera_angle_speed,
            target_pitch_speed: config.motion.camera_pitch_speed,
            direction_timer: 0,
            work_online: Arc::new(AtomicBool::new(false)),
            poll_result: Arc::new(Mutex::new(None)),
            polling: Arc::new(AtomicBool::new(false)),
            config,
        }
    }

    fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.config.timing.poll_interval_secs)
    }

    fn tick_rate(&self) -> Duration {
        Duration::from_millis(self.config.timing.tick_rate_ms)
    }

    fn distribute_3d_positions(&mut self) {
        let count = self.agents.len();
        if count == 0 {
            return;
        }

        let octahedron: Vec<[f64; 3]> = vec![
            [ 0.0,  3.5,  0.0],
            [ 0.0, -3.5,  0.0],
            [ 3.5,  0.0,  0.0],
            [-3.5,  0.0,  0.0],
            [ 0.0,  0.0,  3.5],
            [ 0.0,  0.0, -3.5],
        ];

        for (i, agent) in self.agents.iter_mut().enumerate() {
            if i < octahedron.len() {
                agent.pos3d = octahedron[i];
            } else {
                let golden_ratio = (1.0 + 5.0_f64.sqrt()) / 2.0;
                let theta = std::f64::consts::TAU * i as f64 / golden_ratio;
                let phi = (1.0 - 2.0 * (i as f64 + 0.5) / count as f64).acos();
                agent.pos3d = [
                    2.0 * phi.sin() * theta.cos(),
                    2.0 * phi.sin() * theta.sin(),
                    2.0 * phi.cos(),
                ];
            }
        }

        self.edges.clear();
        for i in 0..count {
            for j in (i + 1)..count {
                self.edges.push((i, j));
            }
        }
    }

    fn project_to_screen(&mut self, screen_width: f64, screen_height: f64) {
        let center_x = screen_width / 2.0;
        let center_y = screen_height / 2.0;
        let scale = screen_width.min(screen_height) * 0.80;
        let camera_distance = self.config.motion.camera_distance;

        let (sa, ca) = self.camera_angle.sin_cos();
        let (sp, cp) = self.camera_pitch.sin_cos();

        let cam_x = camera_distance * cp * sa;
        let cam_y = camera_distance * sp;
        let cam_z = camera_distance * cp * ca;

        let fx = -cp * sa;
        let fy = -sp;
        let fz = -cp * ca;

        let rx = ca;
        let ry = 0.0;
        let rz = -sa;

        let ux = ry * fz - rz * fy;
        let uy = rz * fx - rx * fz;
        let uz = rx * fy - ry * fx;

        for agent in &mut self.agents {
            let dx = agent.pos3d[0] - cam_x;
            let dy = agent.pos3d[1] - cam_y;
            let dz = agent.pos3d[2] - cam_z;

            let depth = dx * fx + dy * fy + dz * fz;
            let screen_right = dx * rx + dy * ry + dz * rz;
            let screen_up = dx * ux + dy * uy + dz * uz;

            let perspective = if depth < 0.1 { 100.0 } else { scale / depth };

            agent.screen_x = center_x + screen_right * perspective;
            agent.screen_y = center_y - screen_up * perspective;
            agent.screen_depth = depth;
        }
    }

    fn start_poll(&mut self) {
        if self.polling.load(Ordering::Relaxed) {
            return;
        }
        self.polling.store(true, Ordering::Relaxed);
        self.last_poll = Instant::now();

        let result_slot = self.poll_result.clone();
        let polling_flag = self.polling.clone();
        let config = self.config.clone();

        std::thread::spawn(move || {
            let poll_result = Self::do_poll(&config);
            if let Ok(mut slot) = result_slot.lock() {
                *slot = Some(poll_result);
            }
            polling_flag.store(false, Ordering::Relaxed);
        });
    }

    fn apply_poll(&mut self) {
        let result = if let Ok(mut slot) = self.poll_result.lock() {
            slot.take()
        } else {
            None
        };

        if let Some(result) = result {
            let old_count = self.agents.len();

            let mut new_agents = result.agents;
            for new_agent in &mut new_agents {
                if let Some(existing) = self.agents.iter().find(|a| a.id == new_agent.id) {
                    new_agent.pulse_phase = existing.pulse_phase;
                    new_agent.pos3d = existing.pos3d;
                }
            }

            self.gateway_online = result.gateway_online;
            self.status_message = result.status_message;
            self.agents = new_agents;

            if old_count != self.agents.len() {
                self.distribute_3d_positions();
            }
        }
    }

    fn do_poll(config: &MeshConfig) -> PollResult {
        let openclaw_dir = dirs::home_dir()
            .map(|h| h.join(".openclaw"))
            .unwrap_or_default();

        let config_path = openclaw_dir.join("openclaw.json");
        let local_agents: Vec<AgentsListEntry> = if let Ok(data) = std::fs::read_to_string(&config_path) {
            if let Ok(openclaw_config) = serde_json::from_str::<serde_json::Value>(&data) {
                if let Some(list) = openclaw_config.get("agents").and_then(|a| a.get("list")).and_then(|l| l.as_array()) {
                    list.iter().filter_map(|entry| {
                        let id = entry.get("id").and_then(|v| v.as_str())?;
                        let model = entry.get("model").and_then(|m| m.get("primary")).and_then(|v| v.as_str());
                        let identity_name = entry.get("identity").and_then(|i| i.get("name")).and_then(|v| v.as_str());
                        let identity_emoji = entry.get("identity").and_then(|i| i.get("emoji")).and_then(|v| v.as_str());
                        Some(AgentsListEntry {
                            id: id.to_string(),
                            identity_name: identity_name.map(|s| s.to_string()),
                            identity_emoji: identity_emoji.map(|s| s.to_string()),
                            model: model.map(|s| s.to_string()),
                        })
                    }).collect()
                } else { vec![] }
            } else { vec![] }
        } else {
            return PollResult {
                agents: vec![],
                gateway_online: false,
                status_message: "config not found".into(),
            };
        };

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let active_threshold_ms = config.timing.active_threshold_minutes * 60 * 1000;

        let mut new_agents: Vec<AgentNode> = vec![];

        for agent in &local_agents {
            let sessions_path = openclaw_dir
                .join("agents")
                .join(&agent.id)
                .join("sessions/sessions.json");

            let sessions_dir = openclaw_dir
                .join("agents")
                .join(&agent.id)
                .join("sessions");
            let newest_transcript_ms = std::fs::read_dir(&sessions_dir)
                .ok()
                .map(|entries| {
                    entries.filter_map(|e| e.ok())
                        .filter(|e| e.path().extension().map(|ext| ext == "jsonl").unwrap_or(false))
                        .filter_map(|e| e.metadata().ok()?.modified().ok())
                        .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64)
                        .max()
                        .unwrap_or(0)
                })
                .unwrap_or(0);

            let (active_count, total_sessions, total_tokens) = if let Ok(data) = std::fs::read_to_string(&sessions_path) {
                if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, serde_json::Value>>(&data) {
                    let mut active = 0u32;
                    let mut tokens = 0u64;
                    let mut count = 0u32;
                    for (key, val) in &map {
                        if key.contains(":run:") { continue; }
                        let updated = val.get("updatedAt").and_then(|v| v.as_u64()).unwrap_or(0);
                        let tok = val.get("totalTokens").and_then(|v| v.as_u64()).unwrap_or(0);
                        count += 1;
                        tokens += tok;
                        let recently_active = now_ms.saturating_sub(updated) < active_threshold_ms
                            || now_ms.saturating_sub(newest_transcript_ms) < active_threshold_ms;
                        if recently_active { active += 1; }
                    }
                    (active, count, tokens)
                } else { (0, 0, 0) }
            } else { (0, 0, 0) };

            new_agents.push(AgentNode {
                id: agent.id.clone(),
                label: agent.identity_name.clone().unwrap_or_else(|| agent.id.clone()),
                emoji: agent.identity_emoji.clone().unwrap_or_else(|| "🤖".into()),
                model: short_model(agent.model.as_deref().unwrap_or("unknown")),
                active: active_count > 0,
                active_sessions: active_count,
                total_sessions,
                total_tokens,
                kind: AgentKind::Local,
                pos3d: [0.0; 3],
                screen_x: 0.0,
                screen_y: 0.0,
                screen_depth: 0.0,
                pulse_phase: 0.0,
                is_local: true,
            });
        }

        let grid_agents = &config.grid;
        let has_ssh = config.connections.ssh_host.is_some() && config.connections.ssh_user.is_some();

        let mut remote_data: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
        let mut work_online = false;

        if has_ssh && !grid_agents.is_empty() {
            let ssh_host = config.connections.ssh_host.as_deref().unwrap();
            let ssh_user = config.connections.ssh_user.as_deref().unwrap();
            let connect_timeout = config.connections.connect_timeout_secs.to_string();
            let ssh_destination = format!("{}@{}", ssh_user, ssh_host);

            let agent_ids: Vec<&str> = grid_agents.iter().map(|a| a.id.as_str()).collect();
            let agent_list_str = agent_ids.iter().map(|id| format!("'{}'", id)).collect::<Vec<_>>().join(",");

            let python_script = format!(
                "python3 -c \"\nimport json,os,time,glob\nnow=time.time()*1000\nagents=[{}]\nresult={{}}\nfor a in agents:\n  sp=os.path.expanduser(f'~/.openclaw/agents/{{a}}/sessions/sessions.json')\n  if not os.path.exists(sp): continue\n  d=json.load(open(sp))\n  sd=os.path.dirname(sp)\n  jfiles=glob.glob(os.path.join(sd,'*.jsonl'))\n  newest_mtime=max((os.path.getmtime(f) for f in jfiles),default=0)*1000\n  active=0;total=0;tokens=0\n  for k,v in d.items():\n    if ':run:' in k: continue\n    total+=1\n    tokens+=v.get('totalTokens',0)\n    updated=v.get('updatedAt',0)\n    if now-updated<{}000 or now-newest_mtime<{}000: active+=1\n  result[a]=dict(active=active,total=total,tokens=tokens)\nprint(json.dumps(result))\n\"",
                agent_list_str,
                active_threshold_ms / 1000,
                active_threshold_ms / 1000,
            );

            let ssh_result = Command::new("ssh")
                .args([
                    "-o", &format!("ConnectTimeout={}", connect_timeout),
                    "-o", "BatchMode=yes",
                    &ssh_destination,
                    &python_script,
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output();

            if let Ok(out) = ssh_result {
                if out.status.success() {
                    work_online = true;
                    if let Ok(data) = serde_json::from_slice(&out.stdout) {
                        remote_data = data;
                    }
                }
            }
        }

        for grid_agent in grid_agents {
            let (active_count, total_sessions, total_tokens) = if let Some(info) = remote_data.get(&grid_agent.id) {
                (
                    info.get("active").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    info.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    info.get("tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                )
            } else {
                (0, 0, 0)
            };

            new_agents.push(AgentNode {
                id: grid_agent.id.clone(),
                label: grid_agent.id.clone(),
                emoji: if grid_agent.emoji.is_empty() { "🤖".into() } else { grid_agent.emoji.clone() },
                model: grid_agent.model.clone(),
                active: active_count > 0,
                active_sessions: active_count,
                total_sessions,
                total_tokens,
                kind: AgentKind::Grid,
                pos3d: [0.0; 3],
                screen_x: 0.0,
                screen_y: 0.0,
                screen_depth: 0.0,
                pulse_phase: 0.0,
                is_local: false,
            });
        }

        let active_count = new_agents.iter().filter(|a| a.active).count();
        let total_count = new_agents.len();
        let work_status = if work_online { "work ⚡" } else if has_ssh { "work ⊘" } else { "local only" };

        PollResult {
            agents: new_agents,
            gateway_online: true,
            status_message: format!("{}/{} active │ {}", active_count, total_count, work_status),
        }
    }

    fn update(&mut self, screen_width: f64, screen_height: f64) {
        self.camera_angle += self.config.motion.camera_angle_speed;
        self.camera_pitch += self.config.motion.camera_pitch_speed;

        if self.camera_angle > std::f64::consts::TAU {
            self.camera_angle -= std::f64::consts::TAU;
        }
        if self.camera_pitch > std::f64::consts::TAU {
            self.camera_pitch -= std::f64::consts::TAU;
        }

        let pulse_speed = self.config.motion.pulse_speed;
        let pulse_decay = self.config.motion.pulse_decay;

        for agent in &mut self.agents {
            if agent.active {
                agent.pulse_phase += pulse_speed;
                if agent.pulse_phase > std::f64::consts::TAU {
                    agent.pulse_phase -= std::f64::consts::TAU;
                }
            } else {
                agent.pulse_phase *= pulse_decay;
            }
        }

        self.project_to_screen(screen_width, screen_height);
        self.tick += 1;
    }
}

fn short_model(model: &str) -> String {
    let name = model.split('/').last().unwrap_or(model);
    match name {
        "claude-opus-4-6" => "opus-4".into(),
        "claude-opus-4-5" => "opus-4.5".into(),
        "claude-sonnet-4-5" => "sonnet-4.5".into(),
        "kimi-k2.5" => "kimi-k2.5".into(),
        other if other.len() > 14 => format!("{}…", &other[..13]),
        other => other.to_string(),
    }
}

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        format!("{}", tokens)
    }
}

fn node_color(agent: &AgentNode, colors: &ColorsConfig) -> Color {
    if !agent.active {
        return color_from_vec(&colors.inactive_node);
    }
    let (r, g, b) = agent.kind.base_color(colors);
    let pulse = ((agent.pulse_phase.sin() + 1.0) / 2.0 * 0.4 + 0.6) as f64;
    Color::Rgb(
        (r as f64 * pulse) as u8,
        (g as f64 * pulse) as u8,
        (b as f64 * pulse) as u8,
    )
}

fn depth_fade(base: Color, depth: f64, strength: f64) -> Color {
    let normalized = ((depth - 4.5) / 7.0).clamp(0.0, 1.0);
    let fade = 1.0 - normalized * strength;
    match base {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f64 * fade) as u8,
            (g as f64 * fade) as u8,
            (b as f64 * fade) as u8,
        ),
        _ => base,
    }
}

fn draw_mesh(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let canvas_width = area.width as f64;
    let canvas_height = area.height as f64 * 2.0;
    let config = &app.config;

    let title = format!(
        " openclaw grid │ {} │ {} ",
        if app.gateway_online { "⚡ online" } else { "⊘ offline" },
        app.status_message
    );

    let agents_clone = app.agents.clone();
    let edges_clone = app.edges.clone();
    let colors = config.colors.clone();
    let motion = config.motion.clone();

    let title_color = color_from_vec(&config.colors.title);
    let border_color = color_from_vec(&config.colors.border);

    let canvas = Canvas::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Line::from(vec![
                    Span::styled(" ◆ ", Style::default().fg(title_color)),
                    Span::styled(title, Style::default().fg(title_color)),
                ]))
                .border_style(Style::default().fg(border_color)),
        )
        .x_bounds([0.0, canvas_width])
        .y_bounds([0.0, canvas_height])
        .paint(move |ctx| {
            for &(from, to) in &edges_clone {
                if let (Some(a), Some(b)) = (agents_clone.get(from), agents_clone.get(to)) {
                    let avg_depth = (a.screen_depth + b.screen_depth) / 2.0;
                    let normalized = ((avg_depth - 4.5) / 7.0).clamp(0.0, 1.0);
                    let fade = 1.0 - normalized * motion.edge_fade_strength;
                    let (base_r, base_g, base_b) = if a.active || b.active {
                        rgb_from_vec(&colors.edge_active)
                    } else {
                        rgb_from_vec(&colors.edge_inactive)
                    };
                    let edge_color = quantize_color(Color::Rgb(
                        (base_r as f64 * fade) as u8,
                        (base_g as f64 * fade) as u8,
                        (base_b as f64 * fade) as u8,
                    ), COLOR_QUANTIZE_STEP);

                    ctx.draw(&ratatui::widgets::canvas::Line {
                        x1: a.screen_x,
                        y1: canvas_height - a.screen_y,
                        x2: b.screen_x,
                        y2: canvas_height - b.screen_y,
                        color: edge_color,
                    });
                }
            }

            let mut sorted_indices: Vec<usize> = (0..agents_clone.len()).collect();
            sorted_indices.sort_by(|a, b| {
                agents_clone[*b].screen_depth.partial_cmp(&agents_clone[*a].screen_depth).unwrap()
            });

            for &idx in &sorted_indices {
                let agent = &agents_clone[idx];
                let sy = canvas_height - agent.screen_y;

                let color = node_color(agent, &colors);
                let color = depth_fade(color, agent.screen_depth, motion.depth_fade_strength);
                let color = quantize_color(color, COLOR_QUANTIZE_STEP);

                let symbol = if agent.active { "◉" } else { "○" };
                ctx.print(agent.screen_x, sy, Span::styled(symbol, Style::default().fg(color)));

                let name_color = if agent.active {
                    color_from_vec(&colors.name_active)
                } else {
                    color_from_vec(&colors.name_inactive)
                };

                if agent.screen_depth < 12.0 {
                    let name_line = format!("{} {}", agent.emoji, agent.label);
                    let name_len = name_line.chars().count();
                    ctx.print(
                        agent.screen_x - (name_len as f64 / 2.0),
                        sy - 2.0,
                        Span::styled(name_line, Style::default().fg(name_color)),
                    );
                }

                if agent.screen_depth < 10.0 {
                    let model_color = if agent.active {
                        color_from_vec(&colors.model_active)
                    } else {
                        color_from_vec(&colors.model_inactive)
                    };
                    let model_len = agent.model.len();
                    ctx.print(
                        agent.screen_x - (model_len as f64 / 2.0),
                        sy - 4.0,
                        Span::styled(agent.model.clone(), Style::default().fg(model_color)),
                    );
                }

                if agent.screen_depth < 8.0 && agent.is_local && agent.total_tokens > 0 {
                    let info = format!("{}tok", format_tokens(agent.total_tokens));
                    let info_len = info.len();
                    let tokens_color = color_from_vec(&colors.tokens);
                    ctx.print(
                        agent.screen_x - (info_len as f64 / 2.0),
                        sy + 2.0,
                        Span::styled(info, Style::default().fg(tokens_color)),
                    );
                }
            }
        });

    frame.render_widget(canvas, area);
}

fn draw_status_bar(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let mut spans = vec![
        Span::styled(" q", Style::default().fg(Color::Rgb(100, 255, 100))),
        Span::styled(" quit ", Style::default().fg(Color::Rgb(0, 200, 200))),
        Span::styled("│", Style::default().fg(Color::Rgb(0, 150, 150))),
        Span::styled(" r", Style::default().fg(Color::Rgb(100, 255, 100))),
        Span::styled(" refresh ", Style::default().fg(Color::Rgb(0, 200, 200))),
        Span::styled("│", Style::default().fg(Color::Rgb(0, 150, 150))),
    ];

    for agent in &app.agents {
        let color = if agent.active {
            Color::Rgb(100, 255, 100)
        } else {
            Color::Rgb(0, 180, 180)
        };
        spans.push(Span::styled(format!(" {}", agent.emoji), Style::default().fg(color)));
        spans.push(Span::styled(
            format!("{} ", agent.label),
            Style::default().fg(if agent.active { Color::Rgb(100, 255, 100) } else { Color::Rgb(0, 180, 180) }),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let active_only = args.iter().any(|a| a == "--active");

    let config = load_mesh_config();

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    stdout().execute(crossterm::terminal::Clear(crossterm::terminal::ClearType::All))?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = App::new(active_only, config);

    loop {
        if app.last_poll.elapsed() >= app.poll_interval() {
            app.start_poll();
        }
        app.apply_poll();

        let size = terminal.size()?;
        app.update(size.width as f64, size.height as f64 * 2.0);

        terminal.draw(|frame| {
            let chunks = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(frame.area());

            draw_mesh(frame, &app, chunks[0]);
            draw_status_bar(frame, &app, chunks[1]);
        })?;

        let tick_rate = app.tick_rate();
        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('r') => {
                            let poll_interval = app.poll_interval();
                            app.last_poll = Instant::now() - poll_interval - Duration::from_secs(1);
                            app.polling.store(false, Ordering::Relaxed);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    Ok(())
}
