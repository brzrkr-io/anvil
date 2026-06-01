// Node 26 defines localStorage and sessionStorage as globals set to undefined.
// This prevents happy-dom's populateGlobal from overriding them (it skips keys
// that already exist in global but are not in the explicit KEYS list).
// This setup file runs in the happy-dom environment and re-installs the
// happy-dom storage objects so tests can call localStorage.clear() etc. directly.
import { beforeEach } from "vitest";

// happy-dom exposes window.localStorage via the Window prototype, but vitest's
// populateGlobal skips it because Node 26 already owns the property. We use a
// Storage in-memory implementation from happy-dom's own Window object if available,
// otherwise fall back to a simple map-backed shim.
function makeStorage() {
  const store = new Map<string, string>();
  return {
    get length() { return store.size; },
    getItem(key: string) { return store.has(key) ? store.get(key)! : null; },
    setItem(key: string, value: string) { store.set(key, String(value)); },
    removeItem(key: string) { store.delete(key); },
    clear() { store.clear(); },
    key(index: number) {
      const keys = [...store.keys()];
      return index < keys.length ? keys[index] : null;
    },
  };
}

const _localStorage = makeStorage();
const _sessionStorage = makeStorage();

Object.defineProperty(globalThis, "localStorage", {
  get() { return _localStorage; },
  configurable: true,
  enumerable: false,
});

Object.defineProperty(globalThis, "sessionStorage", {
  get() { return _sessionStorage; },
  configurable: true,
  enumerable: false,
});

// Clear storage between tests so each test file starts with a clean slate.
// Individual tests that need pre-seeded storage can populate in their own beforeEach.
beforeEach(() => {
  _localStorage.clear();
  _sessionStorage.clear();
});
