;
{}
{ a; b }
var a, b = 1, [c] = d, {e} = f;
let lg = 1, lh;
const li = 2;
if (a) b; else c;
if (a) { b } else if (c) { d } else { e }
for (;;) {}
for (var i = 0; i < 1; i++) {}
for (let i = 0, j = 1; ; ) break;
for (a; b; c);
for (x in y);
for (var x in y);
for (let x in y);
for (const [x, y] of z);
for (x.y of z);
for ([a, b] of c);
for ({a, b} in c);
while (a) continue;
do a; while (b)
do { } while (b);
outer: for (;;) { inner: for (;;) { continue outer; break inner; } }
label: { break label; }
switch (a) { case 1: b; case 2: { c } default: d; case 3: }
switch (a) {}
try { a } catch (e) { b }
try { a } catch { b } finally { c }
try { a } finally { b }
try {} catch ([a, b]) {} catch2: ;
throw new Error('x');
function f() { return; }
function g() { return a, b; }
debugger;
with (a) b;
x = function () { "use strict"; 'other'; "\x75se strict"; }
