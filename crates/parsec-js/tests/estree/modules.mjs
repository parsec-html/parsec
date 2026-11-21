import "side-effect";
import a from "a";
import * as b from "b";
import { c, d as e, "f-g" as h, default as i } from "c";
import j, { k } from "d";
import l, * as m from "e";
import n from "f.json" with { type: "json" };
import { o } from "g" with { "type": "json", other: "x", };
import {} from "h";
export var p = 1, q;
export let r = 2;
export const s = 3;
export function t() {}
export async function u() {}
export function* v() {}
export class W {}
export { a, b as x, c as "y-z", e as default2 };
export { k as "string name" } from "d";
export { "f-g" as fg, default as cDefault } from "c";
export * from "i";
export * as ns from "j";
export * as "string ns" from "k" with { type: "json" };
export {};
export default class {}
await import.meta.resolve("a");
const dyn = await import("b", { with: { type: "json" } });
for await (const y of z) {}
