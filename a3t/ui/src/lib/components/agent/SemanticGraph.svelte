<script module lang="ts">
  export type Fact = { key: string | null; subject: string; predicate: string; object: string; space: string };
</script>

<script lang="ts">
  /// Semantic memory as a graph that turns: each entity (a fact's subject or object)
  /// is a point, each fact a line between them. It rotates on its own; drag to turn
  /// it, scroll to zoom, click an entity to pick it (its facts are listed below).
  import { onMount, onDestroy } from 'svelte';
  import * as THREE from 'three';

  let { facts, selected = '', onselect }: { facts: Fact[]; selected?: string; onselect: (entity: string) => void } = $props();

  let host: HTMLDivElement;
  let tip = $state<{ x: number; y: number; text: string } | null>(null);
  let spinning = $state(true);

  let renderer: THREE.WebGLRenderer | null = null;
  let scene: THREE.Scene, camera: THREE.PerspectiveCamera, world: THREE.Group;
  let nodes: { name: string; deg: number; pos: THREE.Vector3; mesh: THREE.Mesh; label: THREE.Sprite }[] = [];
  let lines: THREE.LineSegments | null = null;
  let anim = 0, ro: ResizeObserver | null = null;
  let yaw = 0, pitch = 0.25, dist = 260, lastTouch = 0;
  const ray = new THREE.Raycaster(), mouse = new THREE.Vector2();

  const css = (v: string, d: string) => (getComputedStyle(host).getPropertyValue(v).trim() || d);

  /// A 3D force layout, run to rest before the first frame.
  function layout(names: string[], links: [number, number][]) {
    const n = names.length, r = 40 + Math.cbrt(n) * 28;
    const p = names.map((_, i) => {
      const t = Math.acos(1 - 2 * (i + 0.5) / n), f = Math.PI * (1 + Math.sqrt(5)) * i;
      return new THREE.Vector3(r * Math.sin(t) * Math.cos(f), r * Math.sin(t) * Math.sin(f), r * Math.cos(t));
    });
    const v = names.map(() => new THREE.Vector3());
    const d = new THREE.Vector3();
    for (let it = 0; it < 260; it++) {
      const k = 1 - it / 260;
      for (let i = 0; i < n; i++) for (let j = i + 1; j < n; j++) {
        d.subVectors(p[i], p[j]); const l2 = Math.max(d.lengthSq(), 1);
        d.multiplyScalar(900 / l2 / Math.sqrt(l2)); v[i].add(d); v[j].sub(d);
      }
      for (const [a, b] of links) {
        d.subVectors(p[b], p[a]); const l = d.length() || 1;
        d.multiplyScalar((l - 34) / l * 0.06); v[a].add(d); v[b].sub(d);
      }
      for (let i = 0; i < n; i++) { v[i].addScaledVector(p[i], -0.004); p[i].addScaledVector(v[i], 0.5 * k); v[i].multiplyScalar(0.6); }
    }
    // Centred, so it turns about its middle.
    const c = new THREE.Vector3(); for (const x of p) c.add(x); c.divideScalar(n || 1);
    for (const x of p) x.sub(c);
    return p;
  }

  function label(text: string, color: string) {
    const c = document.createElement('canvas'), g = c.getContext('2d')!;
    const fs = 28; g.font = `500 ${fs}px system-ui, sans-serif`;
    const w = Math.ceil(g.measureText(text).width) + 12;
    c.width = w; c.height = fs + 12;
    g.font = `500 ${fs}px system-ui, sans-serif`; g.fillStyle = color; g.textBaseline = 'middle'; g.fillText(text, 6, c.height / 2);
    const tex = new THREE.CanvasTexture(c); tex.colorSpace = THREE.SRGBColorSpace;
    const s = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, transparent: true, depthWrite: false }));
    s.scale.set(w / 5.5, c.height / 5.5, 1);
    return s;
  }

  function build() {
    if (!renderer) return;
    world.clear(); nodes = []; lines = null;
    const idx = new Map<string, number>(), names: string[] = [], links: [number, number][] = [];
    const at = (x: string) => { if (!idx.has(x)) { idx.set(x, names.length); names.push(x); } return idx.get(x)!; };
    for (const f of facts) if (f.subject && f.object) links.push([at(f.subject), at(f.object)]);
    if (!names.length) return;
    const pos = layout(names, links);
    const deg = names.map(() => 0); for (const [a, b] of links) { deg[a]++; deg[b]++; }
    const accent = new THREE.Color(css('--spec', '#7c3aed')), dim = new THREE.Color(css('--text-3', '#888'));
    const text = css('--text-1', '#222'), top = [...deg].sort((a, b) => b - a)[Math.min(16, deg.length - 1)] ?? 0;
    names.forEach((name, i) => {
      const hub = deg[i] >= 3;
      const mesh = new THREE.Mesh(new THREE.SphereGeometry(1.6 + Math.min(deg[i], 12) * 0.35, 16, 12),
        new THREE.MeshBasicMaterial({ color: hub ? accent : dim, transparent: true, opacity: 0.95 }));
      mesh.position.copy(pos[i]); mesh.userData.i = i; world.add(mesh);
      const lb = label(name, text); lb.position.copy(pos[i]).add(new THREE.Vector3(0, 4 + deg[i] * 0.2, 0));
      lb.visible = deg[i] >= top || names.length <= 30; world.add(lb);
      nodes.push({ name, deg: deg[i], pos: pos[i], mesh, label: lb });
    });
    const g = new THREE.BufferGeometry().setFromPoints(links.flatMap(([a, b]) => [pos[a], pos[b]]));
    lines = new THREE.LineSegments(g, new THREE.LineBasicMaterial({ color: css('--border-strong', css('--text-4', '#999')), transparent: true, opacity: 0.55 }));
    world.add(lines);
    dist = 90 + Math.cbrt(names.length) * 48;
    mark();
  }

  /// The picked entity and its neighbours stand out; the rest fade.
  function mark() {
    const near = new Set<string>([selected]);
    if (selected) for (const f of facts) { if (f.subject === selected) near.add(f.object); if (f.object === selected) near.add(f.subject); }
    for (const n of nodes) {
      const m = n.mesh.material as THREE.MeshBasicMaterial;
      m.opacity = !selected || near.has(n.name) ? 0.95 : 0.18;
      n.mesh.scale.setScalar(n.name === selected ? 1.8 : 1);
      n.label.visible = selected ? near.has(n.name) : n.deg >= ([...nodes].map((x) => x.deg).sort((a, b) => b - a)[Math.min(16, nodes.length - 1)] ?? 0) || nodes.length <= 30;
    }
    if (lines) (lines.material as THREE.LineBasicMaterial).opacity = selected ? 0.25 : 0.55;
  }

  function pick(e: PointerEvent | MouseEvent) {
    const r = host.getBoundingClientRect();
    mouse.set(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1);
    ray.setFromCamera(mouse, camera);
    const hit = ray.intersectObjects(nodes.map((n) => n.mesh))[0];
    return hit ? nodes[hit.object.userData.i] : null;
  }

  let drag: { x: number; y: number; moved: boolean } | null = null;
  function down(e: PointerEvent) { drag = { x: e.clientX, y: e.clientY, moved: false }; host.setPointerCapture(e.pointerId); }
  function move(e: PointerEvent) {
    if (drag) {
      const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
      if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
      yaw -= dx * 0.008; pitch = Math.max(-1.3, Math.min(1.3, pitch + dy * 0.008));
      drag.x = e.clientX; drag.y = e.clientY; lastTouch = performance.now(); tip = null; return;
    }
    const n = pick(e), r = host.getBoundingClientRect();
    tip = n ? { x: e.clientX - r.left + 12, y: e.clientY - r.top + 12, text: `${n.name} · ${n.deg} fact${n.deg === 1 ? '' : 's'}` } : null;
    host.style.cursor = n ? 'pointer' : 'grab';
  }
  function up(e: PointerEvent) {
    if (drag && !drag.moved) { const n = pick(e); onselect(n && n.name !== selected ? n.name : ''); }
    drag = null;
  }
  function wheel(e: WheelEvent) { e.preventDefault(); dist = Math.max(40, Math.min(1200, dist * (1 + Math.sign(e.deltaY) * 0.1))); lastTouch = performance.now(); }

  function frame() {
    anim = requestAnimationFrame(frame);
    if (spinning && performance.now() - lastTouch > 2500 && !drag) yaw += 0.0035;
    camera.position.set(dist * Math.cos(pitch) * Math.sin(yaw), dist * Math.sin(pitch), dist * Math.cos(pitch) * Math.cos(yaw));
    camera.lookAt(0, 0, 0);
    renderer!.render(scene, camera);
  }

  function size() {
    if (!renderer) return;
    const w = host.clientWidth, h = host.clientHeight;
    renderer.setSize(w, h, false); camera.aspect = w / Math.max(h, 1); camera.updateProjectionMatrix();
  }

  onMount(() => {
    renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
    host.appendChild(renderer.domElement);
    scene = new THREE.Scene(); world = new THREE.Group(); scene.add(world);
    camera = new THREE.PerspectiveCamera(50, 1, 1, 5000);
    ro = new ResizeObserver(size); ro.observe(host); size();
    // Colours follow the theme: rebuild when it flips.
    const mq = matchMedia('(prefers-color-scheme: dark)'); const re = () => build();
    mq.addEventListener('change', re);
    const mo = new MutationObserver(re); mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'class'] });
    frame();
    return () => { mq.removeEventListener('change', re); mo.disconnect(); };
  });
  onDestroy(() => { cancelAnimationFrame(anim); ro?.disconnect(); renderer?.dispose(); renderer = null; });

  $effect(() => { facts; build(); });
  $effect(() => { selected; mark(); });
</script>

<div class="graph" bind:this={host} onpointerdown={down} onpointermove={move} onpointerup={up} onpointerleave={() => (tip = null)} onwheel={wheel} role="img"
  aria-label="Semantic memory as a graph: {facts.length} facts">
  {#if tip}<div class="tip" style="left:{tip.x}px;top:{tip.y}px">{tip.text}</div>{/if}
  <div class="ctl">
    <button class="link" onclick={(e) => { e.stopPropagation(); spinning = !spinning; }} onpointerdown={(e) => e.stopPropagation()}>{spinning ? 'Pause' : 'Rotate'}</button>
    {#if selected}<button class="link" onclick={(e) => { e.stopPropagation(); onselect(''); }} onpointerdown={(e) => e.stopPropagation()}>Show all</button>{/if}
  </div>
  <p class="hint">Drag to turn · scroll to zoom · click an entity</p>
</div>

<style>
  .graph { position: relative; height: 420px; border: 1px solid var(--border); border-radius: 10px; background: var(--bg-surface); overflow: hidden; cursor: grab; touch-action: none; }
  .graph :global(canvas) { width: 100% !important; height: 100% !important; display: block; }
  .tip { position: absolute; pointer-events: none; padding: 3px 8px; border-radius: 6px; background: var(--bg-elevated, var(--bg-surface)); border: 1px solid var(--border); font: 12px var(--font-mono); color: var(--text-1); white-space: nowrap; }
  .ctl { position: absolute; top: 8px; right: 10px; display: flex; gap: 10px; }
  .hint { position: absolute; left: 10px; bottom: 6px; margin: 0; font-size: 11px; color: var(--text-4); pointer-events: none; }
  @media (max-width: 600px) { .graph { height: 300px; } }
</style>
