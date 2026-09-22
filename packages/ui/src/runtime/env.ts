/**
 * True in Vite dev/test builds, statically `false` in production builds
 * (Vite replaces `import.meta.env.DEV`), so dev-only audits tree-shake away.
 */
export const IS_DEV: boolean = import.meta.env.DEV;
