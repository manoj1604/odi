// Starseed's page: the game itself is core.wasm (Rust); this draws what it
// writes with WebGPU (shaders/*.wgsl), passes it the pointer and keys, and
// plays sound. Each stage's music is fetched when the stage begins.
const $ = (id) => document.getElementById(id);
const canvas = $("sky");
const t0 = performance.now();
const tell = (what) => { try { parent.postMessage({ odi: "ready", what }, "*"); } catch {} };

function fail(msg) {
  $("note").innerHTML = msg;
  $("note").classList.remove("hide");
  throw new Error(msg);
}

async function main() {
  if (!navigator.gpu) fail("This browser has no WebGPU.<br><small>Chrome, Edge or Safari 26+, over https or localhost</small>");
  const adapter = await navigator.gpu.requestAdapter();
  if (!adapter) fail("WebGPU is here, but no GPU adapter was given.");
  const [device, core, sprite, sky] = await Promise.all([
    adapter.requestDevice(),
    WebAssembly.instantiateStreaming(fetch("core.wasm"), {}).then((r) => r.instance.exports),
    fetch("shaders/sprite.wgsl").then((r) => r.text()),
    fetch("shaders/sky.wgsl").then((r) => r.text()),
  ]);
  const ctx = canvas.getContext("webgpu");
  const format = navigator.gpu.getPreferredCanvasFormat();
  ctx.configure({ device, format, alphaMode: "opaque" });

  const uniform = device.createBuffer({ size: 16, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
  const max = core.max_instances();
  const instances = device.createBuffer({ size: max * 32, usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST });
  const layout = device.createBindGroupLayout({
    entries: [{ binding: 0, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, buffer: {} }],
  });
  const bind = device.createBindGroup({ layout, entries: [{ binding: 0, resource: { buffer: uniform } }] });
  const pl = device.createPipelineLayout({ bindGroupLayouts: [layout] });
  const skyModule = device.createShaderModule({ code: sky });
  const spriteModule = device.createShaderModule({ code: sprite });
  const [skyPipe, spritePipe] = await Promise.all([
    device.createRenderPipelineAsync({
      layout: pl,
      vertex: { module: skyModule, entryPoint: "vs" },
      fragment: { module: skyModule, entryPoint: "fs", targets: [{ format }] },
    }),
    device.createRenderPipelineAsync({
      layout: pl,
      vertex: {
        module: spriteModule, entryPoint: "vs",
        buffers: [{
          arrayStride: 32, stepMode: "instance",
          attributes: [
            { shaderLocation: 0, offset: 0, format: "float32x2" },
            { shaderLocation: 1, offset: 8, format: "float32" },
            { shaderLocation: 2, offset: 12, format: "float32" },
            { shaderLocation: 3, offset: 16, format: "float32x4" },
          ],
        }],
      },
      fragment: {
        module: spriteModule, entryPoint: "fs",
        targets: [{
          format,
          blend: {
            color: { srcFactor: "one", dstFactor: "one", operation: "add" },
            alpha: { srcFactor: "one", dstFactor: "one", operation: "add" },
          },
        }],
      },
    }),
  ]);

  // Size, and the pointer in world units (y up, height 2).
  let aspect = 1;
  function resize() {
    const dpr = Math.min(devicePixelRatio || 1, 2);
    canvas.width = Math.max(1, Math.round(innerWidth * dpr));
    canvas.height = Math.max(1, Math.round(innerHeight * dpr));
    aspect = canvas.width / canvas.height;
    core.resize(aspect);
  }
  core.start((Math.random() * 0xffffffff) >>> 0 || 1, innerWidth / Math.max(1, innerHeight));
  resize();
  addEventListener("resize", resize);
  const toWorld = (e) => {
    const r = canvas.getBoundingClientRect();
    return [((e.clientX - r.left) / r.width * 2 - 1) * aspect, -((e.clientY - r.top) / r.height * 2 - 1)];
  };
  canvas.addEventListener("pointermove", (e) => core.aim(...toWorld(e)));
  canvas.addEventListener("pointerdown", (e) => {
    core.aim(...toWorld(e));
    sound.wake();
    if (core.over()) restart();
  });
  const keyBits = { ArrowUp: 1, KeyW: 1, ArrowDown: 2, KeyS: 2, ArrowLeft: 4, KeyA: 4, ArrowRight: 8, KeyD: 8 };
  let held = 0;
  addEventListener("keydown", (e) => {
    if (keyBits[e.code]) { held |= keyBits[e.code]; core.keys(held); e.preventDefault(); }
    sound.wake();
    if ((e.code === "Space" || e.code === "Enter") && core.over()) restart();
  });
  addEventListener("keyup", (e) => { if (keyBits[e.code]) { held &= ~keyBits[e.code]; core.keys(held); } });

  function restart() {
    core.start((Math.random() * 0xffffffff) >>> 0 || 1, aspect);
    $("note").classList.add("hide");
    sound.stage(1);
  }

  const uni = new Float32Array(4);
  let last = performance.now(), first = true, hurt = 0, shownScore = -1, shownLives = -1, shownStage = 0;
  function frame(now) {
    const dt = (now - last) / 1000;
    last = now;
    const ev = core.step(dt);
    if (ev & 1) sound.gather();
    if (ev & 2) { sound.hit(); hurt = 1; }
    if (ev & 4) sound.stage(core.stage());
    if (ev & 8) {
      $("note").innerHTML = `The seed is spent: ${core.score()} gathered.<br><small>click or press space to drift again</small>`;
      $("note").classList.remove("hide");
    }
    if (core.score() > 0 && shownScore === 0) $("note").classList.add("hide");
    hurt = Math.max(0, hurt - dt * 2);
    if (core.score() !== shownScore) { shownScore = core.score(); $("score").textContent = shownScore; }
    if (core.lives() !== shownLives) { shownLives = core.lives(); $("lives").textContent = "●".repeat(shownLives); }
    if (core.stage() !== shownStage) { shownStage = core.stage(); $("stage").textContent = `stage ${shownStage}`; }

    uni.set([aspect, now / 1000, core.stage(), hurt]);
    device.queue.writeBuffer(uniform, 0, uni);
    const n = core.instance_count();
    device.queue.writeBuffer(instances, 0, core.memory.buffer, core.instances(), n * 32);
    const enc = device.createCommandEncoder();
    const pass = enc.beginRenderPass({
      colorAttachments: [{ view: ctx.getCurrentTexture().createView(), loadOp: "clear", storeOp: "store", clearValue: [0, 0, 0, 1] }],
    });
    pass.setBindGroup(0, bind);
    pass.setPipeline(skyPipe);
    pass.draw(3);
    pass.setPipeline(spritePipe);
    pass.setVertexBuffer(0, instances);
    pass.draw(6, n);
    pass.end();
    device.queue.submit([enc.finish()]);
    if (first) {
      first = false;
      console.log(`[starseed] first frame ${Math.round(performance.now() - t0)} ms`);
      tell("first frame");
      sound.stage(1); // (fetched now, played once there's a click)
    }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
  window.starseed = { core, device, warp: (s) => { core.warp(s); sound.stage(core.stage()); shownStage = 0; } };
}

// Sound: effects made here; each stage's music fetched when the stage begins.
const sound = (() => {
  let ac = null, gain = null, playing = null, want = 0;
  const music = new Map(); // stage -> Promise<AudioBuffer | ArrayBuffer>
  const fetched = new Map();
  function get(stage) {
    if (!fetched.has(stage)) {
      const t = performance.now();
      fetched.set(stage, fetch(`audio/stage${stage}.wav`).then((r) => r.arrayBuffer()).then((b) => {
        console.log(`[starseed] stage ${stage} music: ${(b.byteLength / 1048576).toFixed(2)} MB in ${Math.round(performance.now() - t)} ms`);
        tell(`stage ${stage} music`);
        return b;
      }));
    }
    return fetched.get(stage);
  }
  async function play(stage) {
    if (!ac) return;
    if (!music.has(stage)) music.set(stage, get(stage).then((b) => ac.decodeAudioData(b.slice(0))));
    const buf = await music.get(stage);
    if (want !== stage) return;
    if (playing) { try { playing.stop(); } catch {} }
    const src = ac.createBufferSource();
    src.buffer = buf;
    src.loop = true;
    src.connect(gain);
    src.start();
    playing = src;
  }
  function blip(f0, f1, dur, type, vol) {
    if (!ac) return;
    const o = ac.createOscillator(), g = ac.createGain(), t = ac.currentTime;
    o.type = type;
    o.frequency.setValueAtTime(f0, t);
    o.frequency.exponentialRampToValueAtTime(f1, t + dur);
    g.gain.setValueAtTime(vol, t);
    g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
    o.connect(g).connect(ac.destination);
    o.start(t);
    o.stop(t + dur);
  }
  return {
    wake() {
      if (ac) return;
      ac = new AudioContext();
      gain = ac.createGain();
      gain.gain.value = 0.35;
      gain.connect(ac.destination);
      if (want) play(want);
    },
    stage(s) { want = s; get(s); play(s); },
    gather() { blip(660 + Math.random() * 220, 1320, 0.12, "sine", 0.12); },
    hit() { blip(180, 50, 0.35, "sawtooth", 0.18); },
  };
})();

main().catch((e) => console.error(e));
