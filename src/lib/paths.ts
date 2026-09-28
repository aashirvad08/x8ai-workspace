// Workspace paths: `/`-separated, relative to the root, `""` for the root itself.

/** `src/app/main.ts` → `main.ts`. */
export function basename(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}

/** `src/app/main.ts` → `src/app`; `main.ts` → `""`. */
export function dirname(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash < 0 ? "" : path.slice(0, slash);
}

export function join(dir: string, name: string): string {
  return dir === "" ? name : `${dir}/${name}`;
}

/** Whether `path` is `ancestor` itself or lies beneath it. The root contains everything. */
export function isWithin(path: string, ancestor: string): boolean {
  return ancestor === "" || path === ancestor || path.startsWith(`${ancestor}/`);
}

/** Moves `path` from under `from` to under `to`, for renamed files and directories. */
export function rebase(path: string, from: string, to: string): string {
  return path === from ? to : `${to}${path.slice(from.length)}`;
}
