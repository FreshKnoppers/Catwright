// Pixel icons and helpers shared by the mockups; the logo matches theme::LOGO.
const ICONS = {
  plus: ["...#...","...#...","...#...","#######","...#...","...#...","...#..."],
  search: [".###...","#...#..","#...#..","#...#..",".###...","....##.",".....##"],
  artifacts: ["####...","#..#...","#..####","####..#","...#..#","...#..#","...####"],
  panel: ["#######","#..#..#","#..#..#","#..#..#","#..#..#","#..#..#","#######"],
  down: [".......",".......","#.....#",".#...#.","..#.#..","...#...","......."],
  right: ["..#....","...#...","....#..",".....#.","....#..","...#...","..#...."],
  file: ["####...","#..##..","#...##.","#....#.","#....#.","#....#.","######."],
  close: ["#.....#",".#...#.","..#.#..","...#...","..#.#..",".#...#.","#.....#"],
  open: ["...####",".....##","....#.#","#..#..#","#......","#......","#####.."],
  branch: [".#...#.",".#...#.",".#..#..",".#.#...",".##....",".#.....",".#....."],
  folder: ["###....","#######","#.....#","#.....#","#.....#","#######","......."],
  gear: ["..#.#..",".#####.","##...##",".#...#.","##...##",".#####.","..#.#.."],
  user: ["..###..","..###..","..###..",".......",".#####.","#######","#######"],
  clock: [".#####.","#..#..#","#..#..#","#..##.#","#.....#","#.....#",".#####."],
  bell: ["...#...","..###..",".#####.",".#####.",".#####.","#######","...#..."],
  comment: ["#######","#.....#","#.###.#","#.....#","#######",".#.....","#......"],
  up: ["...#...","..###..",".#.#.#.","...#...","...#...","...#...","...#..."],
  dots: [".......",".......",".......","#..#..#",".......",".......","......."],
  check: [".......","......#",".....#.","#...#..",".#.#...","..#....","......."],
  shield: ["#######","#.....#","#..#..#","#.###.#","#..#..#",".#...#.","..###.."],
  archive: ["#######","#######",".#...#.",".#.#.#.",".#...#.",".#####.","......."],
  star: ["#..#..#",".#.#.#.","..###..","#######","..###..",".#.#.#.","#..#..#"],
  pin: ["..###..","..###..","..###..",".#####.","...#...","...#...","...#..."],
  warn: ["...#...","..###..","..#.#..",".##.##.",".#####.","##.#.##","#######"],
  code: [".......",".#...#.","#.....#","#..#..#","#.....#",".#...#.","......."],
  pencil: [".....##","....###","...###.","..###..",".###...","##.....","#......"],
  trash: [".#####.","#######",".#...#.",".#.#.#.",".#.#.#.",".#...#.",".#####."],
  eye: [".......","..###..",".#...#.","#..#..#",".#...#.","..###..","......."],
  split: ["#######","#..#..#","#..#..#","#######","#..#..#","#..#..#","#######"],
};
function icon(name, color, cell) {
  const b = ICONS[name]; if (!b) return "";
  const c = cell || 2, w = b[0].length * c, h = b.length * c;
  let r = "";
  b.forEach((row, y) => [...row].forEach((ch, x) => { if (ch === "#") r += `<rect x="${x*c}" y="${y*c}" width="${c}" height="${c}"/>`; }));
  return `<svg width="${w}" height="${h}" fill="${color || "currentColor"}" style="flex:none;display:inline-block;vertical-align:middle">${r}</svg>`;
}
// 3x3 project icon
function proj(colors) {
  let r = "";
  colors.forEach((col, i) => r += `<rect x="${(i%3)*4}" y="${Math.floor(i/3)*4}" width="4" height="4" fill="${col}"/>`);
  return `<svg width="12" height="12" style="flex:none">${r}</svg>`;
}
const P_WS = ["#e10500","#fa500f","#ffd800","#5b9cf8","#fa500f","#ffaf00","#e10500","#ff8205","#3fb950"];
const P_HC = ["#ffaf00","#3fb950","#ffd800","#3fb950","#fa500f","#ffaf00","#ff8205","#ffd800","#5b9cf8"];
const PROJ = { P_WS, P_HC };
function logo() {
  const rows = ["ooooooooo","o.......o","o.y...y.o","o.......o","rrrrrrrrr"];
  const col = { o: "#fa500f", y: "#ffd800", r: "#e10500" };
  let r = "";
  rows.forEach((row, y) => [...row].forEach((ch, x) => { if (col[ch]) r += `<rect x="${x*2}" y="${y*2}" width="2" height="2" fill="${col[ch]}"/>`; }));
  return `<svg width="18" height="10">${r}</svg>`;
}
function busy() {
  return `<span style="display:inline-flex;gap:2px"><i style="width:5px;height:5px;background:#e10500;display:block"></i><i style="width:5px;height:5px;background:#ff8205;display:block"></i><i style="width:5px;height:5px;background:#ffd800;display:block"></i></span>`;
}
function meter(n, filled, size) {
  const s = size || 5;
  let r = `<span class="meter">`;
  for (let i = 0; i < n; i++) r += `<i class="${i < filled ? "f" : ""}" style="width:${s}px;height:${s}px"></i>`;
  return r + "</span>";
}
window.addEventListener("DOMContentLoaded", () => {
  document.querySelectorAll("[data-i]").forEach(el => { el.outerHTML = icon(el.dataset.i, el.dataset.c, +el.dataset.s || 2); });
  document.querySelectorAll("[data-proj]").forEach(el => { el.outerHTML = proj(PROJ[el.dataset.proj]); });
  document.querySelectorAll("[data-logo]").forEach(el => { el.outerHTML = logo(); });
  document.querySelectorAll("[data-busy]").forEach(el => { el.outerHTML = busy(); });
  document.querySelectorAll("[data-meter]").forEach(el => { const [n,f,s] = el.dataset.meter.split(",").map(Number); el.outerHTML = meter(n,f,s); });
});
window.addEventListener("DOMContentLoaded", () => {
  if (location.search.includes("clean")) document.querySelectorAll(".note,.mocktag").forEach(e => e.remove());
});
