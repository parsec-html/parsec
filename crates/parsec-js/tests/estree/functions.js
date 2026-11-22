function a() {}
function* b() { yield 1; }
async function c() { await 1; for await (const x of y) {} }
async function* d() { yield await 1; }
function e(a, b = 1, [c, d], {e, f: g = 2}, ...h) {}
function f(a, b,) {}
function g() { 'use strict'; return this; }
(function () { arguments; })();
x = function* () { yield
  a; };
let h = async (a) => { await a; };
let i = () => ({});
let j = (...args) => args;
let k = ({a, b}, [c]) => a;
yield = 1; await = 2; let async = 3; var of = 4;
function l(eval, arguments) {}
