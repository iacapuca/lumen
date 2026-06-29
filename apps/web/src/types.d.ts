// The embed SDK ships plain JS (no .d.ts). We only use it for its side effect
// (registering the <analytics-dashboard> custom element), so a bare module
// declaration is enough.
declare module '@lumen/embed-sdk/element'
declare module '@lumen/embed-sdk'
