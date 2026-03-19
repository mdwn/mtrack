// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//

type DialogMode = "confirm" | "prompt" | "alert";

interface DialogOptions {
  danger?: boolean;
  confirmLabel?: string;
  cancelLabel?: string;
  placeholder?: string;
  defaultValue?: string;
}

interface DialogState {
  open: boolean;
  mode: DialogMode;
  message: string;
  options: DialogOptions;
  resolve: ((value: boolean | string | null) => void) | null;
}

export const dialogState: DialogState = $state({
  open: false,
  mode: "confirm",
  message: "",
  options: {},
  resolve: null,
});

export function showConfirm(
  message: string,
  options?: DialogOptions,
): Promise<boolean> {
  return new Promise((resolve) => {
    dialogState.open = true;
    dialogState.mode = "confirm";
    dialogState.message = message;
    dialogState.options = options ?? {};
    dialogState.resolve = resolve as (value: boolean | string | null) => void;
  });
}

export function showPrompt(
  message: string,
  options?: DialogOptions,
): Promise<string | null> {
  return new Promise((resolve) => {
    dialogState.open = true;
    dialogState.mode = "prompt";
    dialogState.message = message;
    dialogState.options = options ?? {};
    dialogState.resolve = resolve as (value: boolean | string | null) => void;
  });
}

export function showAlert(
  message: string,
  options?: DialogOptions,
): Promise<void> {
  return new Promise((resolve) => {
    dialogState.open = true;
    dialogState.mode = "alert";
    dialogState.message = message;
    dialogState.options = options ?? {};
    dialogState.resolve = (() => resolve()) as (
      value: boolean | string | null,
    ) => void;
  });
}
