// #6 Live editor buffer mirror — lets the Markdown preview reflect unsaved edits
// without going through disk. The active editor publishes its current text here;
// the preview reads it when the path matches.
import { writable } from "svelte/store";

export const editorLive = writable<{ path: string; text: string } | null>(null);
