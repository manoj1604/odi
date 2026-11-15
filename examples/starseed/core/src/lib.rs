//! Starseed's game: a seed of light drifting through a field of motes.
//! Gather the gold ones, dodge the dark ones. The page (game.js) draws what
//! this writes with WebGPU: one instance (8 floats) per glowing thing.
//!
//! The world is y in [-1, 1], x in [-aspect, aspect]. No allocator tricks,
//! no bindings: plain exported functions over one game kept here.

use std::cell::UnsafeCell;

const MAX_MOTES: usize = 900;
const MAX_DUST: usize = 1400;
const MAX_TRAIL: usize = 48;
const MAX_BURST: usize = 400;
/// x, y, radius, glow, r, g, b, a.
const FLOATS: usize = 8;
const MAX_INSTANCES: usize = MAX_DUST + MAX_MOTES + MAX_TRAIL + MAX_BURST + 1;

pub const EV_GATHER: u32 = 1;
pub const EV_HIT: u32 = 2;
pub const EV_STAGE: u32 = 4;
pub const EV_OVER: u32 = 8;

/// Score at which each stage begins (stage 1 at 0).
const STAGES: [u32; 3] = [0, 25, 60];

#[derive(Clone, Copy, Default)]
struct Mote {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    r: f32,
    dark: bool,
    phase: f32,
}

#[derive(Clone, Copy, Default)]
struct Spark {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    gold: bool,
}

struct Game {
    rng: u32,
    aspect: f32,
    t: f32,
    px: f32,
    py: f32,
    vx: f32,
    vy: f32,
    tx: f32,
    ty: f32,
    pointer: bool,
    keys: u32,
    size: f32,
    score: u32,
    lives: u32,
    stage: u32,
    hurt: f32,
    over: bool,
    motes: Vec<Mote>,
    dust: Vec<(f32, f32, f32, f32)>,
    trail: Vec<(f32, f32)>,
    sparks: Vec<Spark>,
    spawn: f32,
    out: Vec<f32>,
    count: u32,
}

struct Cell(UnsafeCell<Option<Game>>);
// (wasm32-unknown-unknown: one thread.)
unsafe impl Sync for Cell {}
static GAME: Cell = Cell(UnsafeCell::new(None));

fn game() -> &'static mut Game {
    // SAFETY: one thread, and no exported function holds this across another.
    unsafe { (*GAME.0.get()).get_or_insert_with(|| Game::new(1)) }
}

impl Game {
    fn new(seed: u32) -> Game {
        let mut g = Game {
            rng: seed.max(1),
            aspect: 1.6,
            t: 0.0,
            px: 0.0,
            py: 0.0,
            vx: 0.0,
            vy: 0.0,
            tx: 0.0,
            ty: 0.0,
            pointer: false,
            keys: 0,
            size: 0.035,
            score: 0,
            lives: 3,
            stage: 1,
            hurt: 0.0,
            over: false,
            motes: Vec::with_capacity(MAX_MOTES),
            dust: Vec::with_capacity(MAX_DUST),
            trail: Vec::with_capacity(MAX_TRAIL),
            sparks: Vec::with_capacity(MAX_BURST),
            spawn: 0.0,
            out: vec![0.0; MAX_INSTANCES * FLOATS],
            count: 0,
        };
        for _ in 0..MAX_DUST {
            let d =
                (g.range(-1.0, 1.0) * 2.0, g.range(-1.0, 1.0), g.range(0.2, 1.0), g.range(0.0, std::f32::consts::TAU));
            g.dust.push(d);
        }
        for _ in 0..40 {
            g.add_mote(true);
        }
        g
    }

    fn next(&mut self) -> u32 {
        // xorshift32
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (self.next() as f32 / u32::MAX as f32) * (b - a)
    }

    /// How hard this stage is: more dark motes, faster.
    fn pace(&self) -> (f32, f32) {
        match self.stage {
            1 => (0.18, 0.22),
            2 => (0.33, 0.32),
            _ => (0.45 + (self.score.saturating_sub(STAGES[2]) as f32 * 0.004).min(0.2), 0.42),
        }
    }

    fn add_mote(&mut self, anywhere: bool) {
        if self.motes.len() >= MAX_MOTES {
            return;
        }
        let (dark_share, speed) = self.pace();
        let dark = self.range(0.0, 1.0) < dark_share;
        let a = self.aspect + 0.1;
        // From an edge, drifting across; or anywhere (the opening field).
        let (x, y) = if anywhere {
            (self.range(-a, a), self.range(-1.0, 1.0))
        } else {
            match self.next() % 4 {
                0 => (-a, self.range(-1.0, 1.0)),
                1 => (a, self.range(-1.0, 1.0)),
                2 => (self.range(-a, a), -1.1),
                _ => (self.range(-a, a), 1.1),
            }
        };
        let aim = (self.range(-0.5, 0.5) - y).atan2(self.range(-a * 0.5, a * 0.5) - x);
        let s = speed * self.range(0.4, 1.0) * if anywhere { 0.4 } else { 1.0 };
        let r = if dark { self.range(0.025, 0.05) } else { self.range(0.012, 0.022) };
        let phase = self.range(0.0, std::f32::consts::TAU);
        self.motes.push(Mote { x, y, vx: aim.cos() * s, vy: aim.sin() * s, r, dark, phase });
    }

    fn burst(&mut self, x: f32, y: f32, n: usize, gold: bool) {
        for _ in 0..n {
            if self.sparks.len() >= MAX_BURST {
                self.sparks.remove(0);
            }
            let a = self.range(0.0, std::f32::consts::TAU);
            let s = self.range(0.2, 0.9);
            let life = self.range(0.4, 0.9);
            self.sparks.push(Spark { x, y, vx: a.cos() * s, vy: a.sin() * s, life, gold });
        }
    }

    fn step(&mut self, dt: f32) -> u32 {
        let dt = dt.clamp(0.0, 0.05);
        self.t += dt;
        let mut ev = 0;

        // The seed: toward the pointer, or pushed by the keys.
        if !self.over {
            let (mut ax, mut ay) = (0.0, 0.0);
            if self.keys != 0 {
                self.pointer = false;
                let k = |b: u32| if self.keys & b != 0 { 1.0 } else { 0.0 };
                ax = (k(8) - k(4)) * 3.2;
                ay = (k(1) - k(2)) * 3.2;
            } else if self.pointer {
                ax = (self.tx - self.px) * 14.0 - self.vx * 5.0;
                ay = (self.ty - self.py) * 14.0 - self.vy * 5.0;
            }
            self.vx = (self.vx + ax * dt) * (1.0 - 2.2 * dt);
            self.vy = (self.vy + ay * dt) * (1.0 - 2.2 * dt);
            self.px = (self.px + self.vx * dt).clamp(-self.aspect, self.aspect);
            self.py = (self.py + self.vy * dt).clamp(-1.0, 1.0);
        }
        self.trail.insert(0, (self.px, self.py));
        self.trail.truncate(MAX_TRAIL);
        self.hurt = (self.hurt - dt).max(0.0);

        // New motes from the edges.
        self.spawn += dt * (2.5 + self.stage as f32 * 1.5);
        while self.spawn >= 1.0 {
            self.spawn -= 1.0;
            self.add_mote(false);
        }

        // Motes drift (a little swirl), and meet the seed.
        let a = self.aspect + 0.2;
        let mut i = 0;
        while i < self.motes.len() {
            let m = &mut self.motes[i];
            let swirl = (self.t * 0.7 + m.phase).sin() * 0.05;
            m.x += (m.vx - m.vy * swirl) * dt;
            m.y += (m.vy + m.vx * swirl) * dt;
            let gone = m.x.abs() > a || m.y.abs() > 1.25;
            let (dx, dy) = (m.x - self.px, m.y - self.py);
            let touch = !self.over && dx * dx + dy * dy < (m.r + self.size).powi(2);
            if touch {
                let (x, y, dark) = (m.x, m.y, m.dark);
                self.motes.swap_remove(i);
                if dark {
                    if self.hurt <= 0.0 {
                        self.lives = self.lives.saturating_sub(1);
                        self.hurt = 1.2;
                        self.size = (self.size * 0.85).max(0.03);
                        ev |= EV_HIT;
                        self.burst(x, y, 40, false);
                        if self.lives == 0 {
                            self.over = true;
                            ev |= EV_OVER;
                            let (px, py) = (self.px, self.py);
                            self.burst(px, py, 160, true);
                        }
                    }
                } else {
                    self.score += 1;
                    self.size = (self.size + 0.0012).min(0.07);
                    ev |= EV_GATHER;
                    self.burst(x, y, 14, true);
                    let next = STAGES.iter().filter(|&&s| self.score >= s).count() as u32;
                    if next > self.stage {
                        self.stage = next;
                        ev |= EV_STAGE;
                    }
                }
                continue;
            }
            if gone {
                self.motes.swap_remove(i);
                continue;
            }
            i += 1;
        }

        for s in &mut self.sparks {
            s.x += s.vx * dt;
            s.y += s.vy * dt;
            s.vx *= 1.0 - 2.0 * dt;
            s.vy *= 1.0 - 2.0 * dt;
            s.life -= dt;
        }
        self.sparks.retain(|s| s.life > 0.0);

        self.write();
        ev
    }

    fn write(&mut self) {
        let mut n = 0;
        let out = &mut self.out;
        let mut put = |v: [f32; FLOATS]| {
            out[n * FLOATS..(n + 1) * FLOATS].copy_from_slice(&v);
            n += 1;
        };
        let (t, stage) = (self.t, self.stage as f32);
        // Dust: far, slow parallax against the seed's drift; tinted by stage.
        let tint = match self.stage {
            1 => [0.55, 0.7, 1.0],
            2 => [0.75, 0.55, 1.0],
            _ => [1.0, 0.55, 0.6],
        };
        for &(x, y, depth, ph) in &self.dust {
            let w = 2.0 * self.aspect + 0.2;
            let dx =
                ((x - self.px * 0.08 * depth + t * 0.01 * depth + self.aspect + 0.1).rem_euclid(w)) - self.aspect - 0.1;
            let dy = ((y - self.py * 0.08 * depth + 1.0).rem_euclid(2.0)) - 1.0;
            let tw = 0.5 + 0.5 * (t * 1.7 + ph).sin();
            put([dx, dy, 0.003 + 0.004 * depth, 0.4, tint[0], tint[1], tint[2], (0.15 + 0.35 * tw) * depth]);
        }
        for m in &self.motes {
            let pulse = 0.85 + 0.15 * (t * 4.0 + m.phase).sin();
            if m.dark {
                put([m.x, m.y, m.r * pulse, 0.25, 0.55, 0.12, 0.45 + 0.1 * stage, 0.95]);
            } else {
                put([m.x, m.y, m.r * pulse, 1.0, 1.0, 0.78, 0.32, 1.0]);
            }
        }
        for (k, &(x, y)) in self.trail.iter().enumerate().skip(1) {
            let f = 1.0 - k as f32 / MAX_TRAIL as f32;
            put([x, y, self.size * 0.7 * f, 0.8, 0.5, 0.9, 1.0, 0.25 * f]);
        }
        for s in &self.sparks {
            let c = if s.gold { [1.0, 0.85, 0.4] } else { [0.8, 0.3, 1.0] };
            put([s.x, s.y, 0.006 + 0.01 * s.life, 1.0, c[0], c[1], c[2], s.life.min(1.0)]);
        }
        if !self.over {
            let blink = if self.hurt > 0.0 && (self.hurt * 12.0).sin() > 0.0 { 0.3 } else { 1.0 };
            put([self.px, self.py, self.size, 1.0, 0.75, 0.95, 1.0, blink]);
        }
        self.count = n as u32;
    }
}

#[no_mangle]
pub extern "C" fn start(seed: u32, aspect: f32) {
    // SAFETY: as in game().
    unsafe { *GAME.0.get() = Some(Game::new(seed)) };
    resize(aspect);
}

#[no_mangle]
pub extern "C" fn resize(aspect: f32) {
    game().aspect = aspect.clamp(0.3, 4.0);
}

/// The pointer, in world units.
#[no_mangle]
pub extern "C" fn aim(x: f32, y: f32) {
    let g = game();
    g.tx = x;
    g.ty = y;
    g.pointer = true;
}

/// Bits: 1 up, 2 down, 4 left, 8 right.
#[no_mangle]
pub extern "C" fn keys(bits: u32) {
    game().keys = bits;
}

/// Advances the game by dt seconds; what happened (EV_* bits).
#[no_mangle]
pub extern "C" fn step(dt: f32) -> u32 {
    game().step(dt)
}

#[no_mangle]
pub extern "C" fn instances() -> *const f32 {
    game().out.as_ptr()
}

#[no_mangle]
pub extern "C" fn instance_count() -> u32 {
    game().count
}

#[no_mangle]
pub extern "C" fn max_instances() -> u32 {
    MAX_INSTANCES as u32
}

#[no_mangle]
pub extern "C" fn score() -> u32 {
    game().score
}

#[no_mangle]
pub extern "C" fn lives() -> u32 {
    game().lives
}

#[no_mangle]
pub extern "C" fn stage() -> u32 {
    game().stage
}

/// Jumps to a stage (1-3), as if its score were reached: for trying stages.
#[no_mangle]
pub extern "C" fn warp(stage: u32) {
    let g = game();
    let s = stage.clamp(1, STAGES.len() as u32);
    g.score = g.score.max(STAGES[s as usize - 1]);
    g.stage = s;
}

#[no_mangle]
pub extern "C" fn over() -> u32 {
    game().over as u32
}
