// Starseed's sky: a slow nebula behind the motes, coloured by the stage.
struct Frame {
  aspect: f32,
  time: f32,
  stage: f32,
  hurt: f32,
};
@group(0) @binding(0) var<uniform> frame: Frame;

struct Out {
  @builtin(position) clip: vec4f,
  @location(0) p: vec2f,
};

@vertex
fn vs(@builtin(vertex_index) v: u32) -> Out {
  // One triangle over the whole screen.
  let xy = vec2f(f32((v << 1u) & 2u), f32(v & 2u)) * 2.0 - 1.0;
  var o: Out;
  o.clip = vec4f(xy, 0.0, 1.0);
  o.p = vec2f(xy.x * frame.aspect, xy.y);
  return o;
}

fn hash(p: vec2f) -> f32 {
  return fract(sin(dot(p, vec2f(127.1, 311.7))) * 43758.5453);
}

fn noise(p: vec2f) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  return mix(mix(hash(i), hash(i + vec2f(1.0, 0.0)), u.x),
             mix(hash(i + vec2f(0.0, 1.0)), hash(i + vec2f(1.0, 1.0)), u.x), u.y);
}

fn fbm(p0: vec2f) -> f32 {
  var p = p0;
  var v = 0.0;
  var a = 0.5;
  for (var k = 0; k < 5; k++) {
    v += a * noise(p);
    p = p * 2.03 + vec2f(1.7, 9.2);
    a *= 0.5;
  }
  return v;
}

@fragment
fn fs(o: Out) -> @location(0) vec4f {
  let t = frame.time * 0.03;
  let q = vec2f(fbm(o.p * 1.3 + vec2f(t, -t)), fbm(o.p * 1.3 + vec2f(-t, t * 0.7) + 4.0));
  let n = fbm(o.p * 1.6 + q * 1.8);
  var deep = vec3f(0.02, 0.03, 0.08);
  var cloud = vec3f(0.10, 0.22, 0.42);
  if (frame.stage >= 2.0) { deep = vec3f(0.04, 0.02, 0.09); cloud = vec3f(0.30, 0.12, 0.45); }
  if (frame.stage >= 3.0) { deep = vec3f(0.07, 0.02, 0.04); cloud = vec3f(0.48, 0.12, 0.20); }
  var c = mix(deep, cloud, smoothstep(0.35, 0.9, n));
  c += vec3f(0.5, 0.1, 0.2) * frame.hurt * 0.25;
  let vignette = 1.0 - 0.35 * dot(o.p / vec2f(frame.aspect, 1.0), o.p / vec2f(frame.aspect, 1.0));
  return vec4f(c * vignette, 1.0);
}
