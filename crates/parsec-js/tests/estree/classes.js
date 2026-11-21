class A {}
class B extends A { constructor() { super(); super.x; super[y]; } }
class C {
  a; b = 1; static c; static d = 2; 'e' = 3; [f] = 4; 5 = 6;
  #g; #h = 1; static #i;
  m() {} static n() {} *o() {} async p() {} async *q() {}
  get r() { return this.#g; } set r(v) {} static get s() {} static set s(v) {}
  #t() {} get #u() {} set #u(v) {} static #v() {}
  [w]() {} 'x'() {} 7() {}
  static { this.y = 1; }
  static {}
  z() { return #g in this; }
  static = 1;
  get;
  set;
  async;
  constructor(a) {}
}
x = class { static async *[Symbol.iterator]() {} };
class D extends (a, b) {}
class E extends f() {}
class F extends G.H {}
