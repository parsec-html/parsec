[a, b] = c;
[a, , b] = c;
[a = 1, [b], {c}, ...d] = e;
({ a, b: c, d = 1, e: f = 2, [g]: h, ...i } = j);
({ a: b.c, d: e[f] } = g);
[a.b, c[d], ...e.f] = g;
[(a), (b.c)] = d;
({ a: (b) } = c);
let [x, y = 1, , ...z] = w;
let { p, q: r, s = 2, t: { u }, ...v } = w;
function f([a, b], { c, d = 1 }) {}
try {} catch ({ message }) {}
for (const { a, b } of c) {}
(a) = 1; (a.b) = 2;
[[[a]]] = b;
({ a: { b: { c } } } = d);
