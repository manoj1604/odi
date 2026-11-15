// Starseed: every glowing thing is one instance of a quad, drawn additively.
struct Frame {
  aspect: f32,
  time: f32,
  stage: f32,
  hurt: f32,
};
@group(0) @binding(0) var<uniform> frame: Frame;

struct In {
  @builtin(vertex_index) v: u32,
  @location(0) pos: vec2f,
  @location(1) radius: f32,
  @location(2) glow: f32,
  @location(3) color: vec4f,
};

struct Out {
  @builtin(position) clip: vec4f,
  @location(0) uv: vec2f,
  @location(1) color: vec4f,
  @location(2) glow: f32,
  @location(3) halo: f32,
};

const corners = array<vec2f, 6>(
  vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(-1.0, 1.0),
  vec2f(-1.0, 1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
);

@vertex
fn vs(i: In) -> Out {
  // The quad reaches past the core so the halo has room.
  let halo = 1.0 + i.glow * 3.0;
  let c = corners[i.v];
  let world = i.pos + c * i.radius * halo;
  var o: Out;
  o.clip = vec4f(world.x / frame.aspect, world.y, 0.0, 1.0);
  o.uv = c * halo;
  o.color = i.color;
  o.glow = i.glow;
  o.halo = halo;
  return o;
}

@fragment
fn fs(o: Out) -> @location(0) vec4f {
  let d = length(o.uv);
  let core = 1.0 - smoothstep(0.7, 1.0, d);
  let glow = o.glow * 0.55 * exp(-2.2 * max(d - 0.6, 0.0));
  let a = clamp(core + glow, 0.0, 1.0) * o.color.a * (1.0 - smoothstep(o.halo * 0.85, o.halo, d));
  let white = core * 0.45 * o.glow;
  return vec4f((o.color.rgb + vec3f(white)) * a, a);
}
