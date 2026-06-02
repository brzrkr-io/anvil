import { writable } from "svelte/store";

type TextRequest = {
  kind: "text";
  title: string;
  message?: string;
  value?: string;
  placeholder?: string;
  okLabel?: string;
  resolve: (v: string | null) => void;
};

type ConfirmRequest = {
  kind: "confirm";
  title: string;
  message?: string;
  okLabel?: string;
  danger?: boolean;
  resolve: (v: boolean) => void;
};

export type DialogRequest = TextRequest | ConfirmRequest;

export const activeDialog = writable<DialogRequest | null>(null);

export function askText(opts: {
  title: string;
  message?: string;
  value?: string;
  placeholder?: string;
  okLabel?: string;
}): Promise<string | null> {
  return new Promise((resolve) => {
    activeDialog.set({ kind: "text", ...opts, resolve });
  });
}

export function askConfirm(opts: {
  title: string;
  message?: string;
  okLabel?: string;
  danger?: boolean;
}): Promise<boolean> {
  return new Promise((resolve) => {
    activeDialog.set({ kind: "confirm", ...opts, resolve });
  });
}
