// #79 Offline / degraded-mode. A single source of truth for connectivity so the
// UI can pause or warn on network features (LLM, k8s, observability) instead of
// hanging. Driven by the browser online/offline events.
import { writable } from "svelte/store";

export const online = writable<boolean>(typeof navigator === "undefined" ? true : navigator.onLine);

if (typeof window !== "undefined") {
  window.addEventListener("online", () => online.set(true));
  window.addEventListener("offline", () => online.set(false));
}
