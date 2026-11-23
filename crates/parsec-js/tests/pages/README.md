# Page state from real pages

Each file is one inline `<script>` from a public page, cut down to a few
of its real keys and values, for the tests in `tests/data.rs`. They were
fetched with `curl` on 21 November 2026. The pages' content belongs to
their owners and none states a licence, so only short excerpts are kept,
with tokens, keys and anything that looked like a credential left out.

| File | Shape | From |
|---|---|---|
| `nextjs-app-router.js` | Next.js App Router: `self.__next_f.push([1, "…"])` | https://nextjs.org/: the bootstrap push and the start of the first payload push, cut in two mid-row the way Next.js cuts its pushes |
| `next-data-hulu.json` | Next.js Pages Router: the JSON in `<script id="__NEXT_DATA__">` | https://www.hulu.com/: seven of its fields |
| `nuxt2-backmarket.js` | Nuxt 2: `window.__NUXT__=(function(a,b,…){return {…}}(…))` | https://www.backmarket.com/ as the Wayback Machine archived it on 5 January 2022 (https://web.archive.org/web/20220105152636/https://www.backmarket.com/), when it ran Nuxt 2: the first 31 parameters and arguments, unchanged, and a few of the returned object's keys that use them |
| `nuxt3-gitlab.js` | Nuxt 3: `window.__NUXT__={};window.__NUXT__.config={…}` | https://gitlab.com/ |
| `redux-lowes.js` | A Redux store: `window['__PRELOADED_STATE__'] = {…}` | https://www.lowes.com/ |
| `json-parse-target.js` | `window.__FLAGS__ = JSON.parse("…")` | https://www.target.com/: the first five flags |
| `var-youtube.js` | A top-level `var ytInitialData = {…}` | https://www.youtube.com/ |
