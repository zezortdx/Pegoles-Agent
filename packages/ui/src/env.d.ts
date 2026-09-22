// Minimal Vite `import.meta` typing for this package's own typecheck.
// Consumers built with Vite get the full types from `vite/client`.
interface ImportMetaEnv {
  readonly DEV: boolean;
  readonly PROD: boolean;
  readonly MODE: string;
}

interface ImportMetaGlobOptions {
  readonly eager?: boolean;
  readonly query?: string;
  readonly import?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
  glob<T = unknown>(pattern: string | readonly string[], options?: ImportMetaGlobOptions): Record<string, T>;
}
