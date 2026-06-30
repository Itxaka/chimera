// Dynamic per-VM route: SPA only (no prerender — ids are unbounded; served via
// the adapter-static index.html fallback and rendered client-side).
export const prerender = false;
export const ssr = false;
