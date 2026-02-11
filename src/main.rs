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
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant},
};

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const TICK_RATE: Duration = Duration::from_millis(33);
const ACTIVE_THRESHOLD_MINUTES: u64 = 10;
const CAMERA_SPEED: f64 = 0.020;
const CAMERA_DISTANCE: f64 = 8.0;

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
    fn base_color(&self) -> (u8, u8, u8) {
        match self {
            AgentKind::Local => (50, 255, 50),
            AgentKind::Grid => (50, 200, 255),
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
}

impl App {
    fn new(active_only: bool) -> Self {
        Self {
            agents: vec![],
            edges: vec![],
            tick: 0,
            last_poll: Instant::now() - POLL_INTERVAL - Duration::from_secs(1),
            gateway_online: false,
            status_message: String::from("connecting..."),
            show_active_only: active_only,
            camera_angle: 0.0,
            camera_pitch: 0.25,
            angle_speed: 0.020,
            pitch_speed: 0.0,
            target_angle_speed: 0.020,
            target_pitch_speed: 0.005,
            direction_timer: 0,
            work_online: Arc::new(AtomicBool::new(false)),
        }
    }

    fn distribute_3d_positions(&mut self) {
        let count = self.agents.len();
        if count == 0 {
            return;
        }

        // Octahedron vertices — perfect for 6 nodes, very stable geometry
        let octahedron: Vec<[f64; 3]> = vec![
            [ 0.0,  3.5,  0.0],  // top
            [ 0.0, -3.5,  0.0],  // bottom
            [ 3.5,  0.0,  0.0],  // right
            [-3.5,  0.0,  0.0],  // left
            [ 0.0,  0.0,  3.5],  // front
            [ 0.0,  0.0, -3.5],  // back
        ];

        for (i, agent) in self.agents.iter_mut().enumerate() {
            if i < octahedron.len() {
                agent.pos3d = octahedron[i];
            } else {
                // Extra nodes go on a larger sphere
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

        // Full mesh edges
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

        // Build camera rotation matrix from two angles
        // Rotate around Y (horizontal) then X (pitch) — no gimbal lock
        let (sa, ca) = self.camera_angle.sin_cos();
        let (sp, cp) = self.camera_pitch.sin_cos();

        // Camera position on sphere looking at origin
        let cam_x = CAMERA_DISTANCE * cp * sa;
        let cam_y = CAMERA_DISTANCE * sp;
        let cam_z = CAMERA_DISTANCE * cp * ca;

        // Forward (toward origin)
        let fx = -cp * sa;
        let fy = -sp;
        let fz = -cp * ca;

        // Right (always horizontal)
        let rx = ca;
        let ry = 0.0;
        let rz = -sa;

        // Up (cross right x forward)
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

    fn poll_data(&mut self) {
        let openclaw_dir = dirs::home_dir()
            .map(|h| h.join(".openclaw"))
            .unwrap_or_default();

        // Read agents list from cached config (avoid CLI call)
        let config_path = openclaw_dir.join("openclaw.json");
        let local_agents: Vec<AgentsListEntry> = if let Ok(data) = std::fs::read_to_string(&config_path) {
            if let Ok(config) = serde_json::from_str::<serde_json::Value>(&data) {
                self.gateway_online = true;
                if let Some(list) = config.get("agents").and_then(|a| a.get("list")).and_then(|l| l.as_array()) {
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
            self.gateway_online = false;
            self.status_message = "config not found".into();
            self.last_poll = Instant::now();
            return;
        };

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let active_threshold_ms = ACTIVE_THRESHOLD_MINUTES * 60 * 1000;

        let mut new_agents: Vec<AgentNode> = vec![];
        let old_count = self.agents.len();

        for agent in &local_agents {
            let sessions_path = openclaw_dir
                .join("agents")
                .join(&agent.id)
                .join("sessions/sessions.json");

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
                        if now_ms.saturating_sub(updated) < active_threshold_ms { active += 1; }
                    }
                    (active, count, tokens)
                } else { (0, 0, 0) }
            } else { (0, 0, 0) };

            let active = active_count > 0;
            let existing = self.agents.iter().find(|n| n.id == agent.id);
            let pulse_phase = existing.map(|n| n.pulse_phase).unwrap_or(0.0);
            let pos3d = existing.map(|n| n.pos3d).unwrap_or([0.0; 3]);

            new_agents.push(AgentNode {
                id: agent.id.clone(),
                label: agent.identity_name.clone().unwrap_or_else(|| agent.id.clone()),
                emoji: agent.identity_emoji.clone().unwrap_or_else(|| "🤖".into()),
                model: short_model(agent.model.as_deref().unwrap_or("unknown")),
                active,
                active_sessions: active_count,
                total_sessions,
                total_tokens,
                kind: AgentKind::Local,
                pos3d,
                screen_x: 0.0,
                screen_y: 0.0,
                screen_depth: 0.0,
                pulse_phase,
                is_local: true,
            });
        }

        // Grid agents (work machine via SSH)
        let grid_agents_meta = vec![
            ("robson", "⚽", "sonnet-4.5"),
            ("jenny", "🎀", "kimi-k2.5"),
            ("monster", "👾", "kimi-k2.5"),
            ("silver", "🪙", "kimi-k2.5"),
        ];

        // SSH to work machine and get session freshness (background-cached)
        let work_flag = self.work_online.clone();
        let grid_data = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<String, (u32, u32, u64)>::new()));
        let grid_data_clone = grid_data.clone();

        let ssh_result = Command::new("ssh")
            .args(["-o", "ConnectTimeout=2", "-o", "BatchMode=yes", "lucas.zanoni@100.127.240.60",
                "python3 -c \"import json,sys,os,time; now=time.time()*1000; \
                agents=['robson','jenny','monster','silver']; \
                result={}; \
                [result.update({a: dict(active=sum(1 for k,v in json.load(open(os.path.expanduser(f'~/.openclaw/agents/{a}/sessions/sessions.json'))).items() if ':run:' not in k and now-v.get('updatedAt',0)<600000), \
                total=sum(1 for k in json.load(open(os.path.expanduser(f'~/.openclaw/agents/{a}/sessions/sessions.json'))).keys() if ':run:' not in k), \
                tokens=sum(v.get('totalTokens',0) for k,v in json.load(open(os.path.expanduser(f'~/.openclaw/agents/{a}/sessions/sessions.json'))).items() if ':run:' not in k))}) for a in agents if os.path.exists(os.path.expanduser(f'~/.openclaw/agents/{a}/sessions/sessions.json'))]; \
                print(json.dumps(result))\""])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();

        let mut remote_data: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
        match ssh_result {
            Ok(out) if out.status.success() => {
                work_flag.store(true, Ordering::Relaxed);
                if let Ok(data) = serde_json::from_slice::<std::collections::HashMap<String, serde_json::Value>>(&out.stdout) {
                    remote_data = data;
                }
            }
            _ => {
                work_flag.store(false, Ordering::Relaxed);
            }
        }
        let work_online = self.work_online.load(Ordering::Relaxed);

        for (agent_id, emoji, model) in &grid_agents_meta {
            let (active_count, total_sessions, total_tokens) = if let Some(info) = remote_data.get(*agent_id) {
                (
                    info.get("active").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    info.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    info.get("tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                )
            } else {
                (0, 0, 0)
            };

            let active = active_count > 0;
            let existing = self.agents.iter().find(|n| n.id == *agent_id);
            let pulse_phase = existing.map(|n| n.pulse_phase).unwrap_or(0.0);
            let pos3d = existing.map(|n| n.pos3d).unwrap_or([0.0; 3]);

            new_agents.push(AgentNode {
                id: agent_id.to_string(),
                label: agent_id.to_string(),
                emoji: emoji.to_string(),
                model: model.to_string(),
                active,
                active_sessions: active_count,
                total_sessions,
                total_tokens,
                kind: AgentKind::Grid,
                pos3d,
                screen_x: 0.0,
                screen_y: 0.0,
                screen_depth: 0.0,
                pulse_phase,
                is_local: false,
            });
        }

        let active_count = new_agents.iter().filter(|a| a.active).count();
        let total_count = new_agents.len();
        let work_status = if work_online { "work ⚡" } else { "work ⊘" };
        self.status_message = format!("{}/{} active │ {}", active_count, total_count, work_status);

        let needs_layout = old_count != new_agents.len();
        self.agents = new_agents;

        if needs_layout {
            self.distribute_3d_positions();
        }

        self.last_poll = Instant::now();
    }

    fn update(&mut self, screen_width: f64, screen_height: f64) {
        self.camera_angle += 0.040;
        // Full 360 pitch rotation
        self.camera_pitch += 0.012;

        if self.camera_angle > std::f64::consts::TAU {
            self.camera_angle -= std::f64::consts::TAU;
        }
        if self.camera_pitch > std::f64::consts::TAU {
            self.camera_pitch -= std::f64::consts::TAU;
        }

        for agent in &mut self.agents {
            if agent.active {
                agent.pulse_phase += 0.15;
                if agent.pulse_phase > std::f64::consts::TAU {
                    agent.pulse_phase -= std::f64::consts::TAU;
                }
            } else {
                agent.pulse_phase *= 0.95;
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

fn node_color(agent: &AgentNode) -> Color {
    if !agent.active {
        return Color::Rgb(40, 60, 40);
    }
    let (r, g, b) = agent.kind.base_color();
    let pulse = ((agent.pulse_phase.sin() + 1.0) / 2.0 * 0.4 + 0.6) as f64;
    Color::Rgb(
        (r as f64 * pulse) as u8,
        (g as f64 * pulse) as u8,
        (b as f64 * pulse) as u8,
    )
}

fn depth_fade(base: Color, depth: f64) -> Color {
    let normalized = ((depth - 4.5) / 7.0).clamp(0.0, 1.0);
    let fade = 1.0 - normalized * 0.75;
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

    let title = format!(
        " openclaw grid │ {} │ {} ",
        if app.gateway_online { "⚡ online" } else { "⊘ offline" },
        app.status_message
    );

    let agents_clone = app.agents.clone();
    let edges_clone = app.edges.clone();

    let canvas = Canvas::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Line::from(vec![
                    Span::styled(" ◆ ", Style::default().fg(Color::Rgb(50, 255, 50))),
                    Span::styled(title, Style::default().fg(Color::Rgb(50, 255, 50))),
                ]))
                .border_style(Style::default().fg(Color::Rgb(0, 100, 0))),
        )
        .x_bounds([0.0, canvas_width])
        .y_bounds([0.0, canvas_height])
        .paint(move |ctx| {
            // Draw edges (back to front for correct overlap)
            for &(from, to) in &edges_clone {
                if let (Some(a), Some(b)) = (agents_clone.get(from), agents_clone.get(to)) {
                    let avg_depth = (a.screen_depth + b.screen_depth) / 2.0;
                    // depth ~4.5 = front, ~11.5 = back (cam at 8, radius 3.5)
                    let normalized = ((avg_depth - 4.5) / 7.0).clamp(0.0, 1.0);
                    let fade = 1.0 - normalized * 0.85;
                    let (r, g, b_val) = if a.active || b.active {
                        (0, (255.0 * fade) as u8, (255.0 * fade) as u8)
                    } else {
                        (0, (220.0 * fade) as u8, (240.0 * fade) as u8)
                    };

                    ctx.draw(&ratatui::widgets::canvas::Line {
                        x1: a.screen_x,
                        y1: canvas_height - a.screen_y,
                        x2: b.screen_x,
                        y2: canvas_height - b.screen_y,
                        color: Color::Rgb(r, g, b_val),
                    });
                }
            }

            // Sort agents by depth (far first) for painter's algorithm
            let mut sorted_indices: Vec<usize> = (0..agents_clone.len()).collect();
            sorted_indices.sort_by(|a, b| {
                agents_clone[*b].screen_depth.partial_cmp(&agents_clone[*a].screen_depth).unwrap()
            });

            for &idx in &sorted_indices {
                let agent = &agents_clone[idx];
                let sy = canvas_height - agent.screen_y;

                let color = node_color(agent);
                let color = depth_fade(color, agent.screen_depth);

                // Node symbol — size based on depth
                let symbol = if agent.active { "◉" } else { "○" };
                ctx.print(agent.screen_x, sy, Span::styled(symbol, Style::default().fg(color)));

                // Depth zones: close (<4) = full info, medium (<6) = name only, far = hidden
                let name_color = if agent.active {
                    Color::Rgb(100, 255, 100)
                } else {
                    Color::Rgb(0, 150, 0)
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
                        Color::Rgb(255, 255, 100)
                    } else {
                        Color::Rgb(150, 150, 50)
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
                    ctx.print(
                        agent.screen_x - (info_len as f64 / 2.0),
                        sy + 2.0,
                        Span::styled(info, Style::default().fg(Color::Rgb(150, 220, 255))),
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

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    stdout().execute(crossterm::terminal::Clear(crossterm::terminal::ClearType::All))?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = App::new(active_only);

    loop {
        if app.last_poll.elapsed() >= POLL_INTERVAL {
            app.poll_data();
        }

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

        if event::poll(TICK_RATE)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('r') => {
                            app.last_poll = Instant::now() - POLL_INTERVAL - Duration::from_secs(1);
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
