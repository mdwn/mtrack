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

import { waitLocale } from "./lib/i18n";
// The player's rules module: loaded before anything can ask it a question.
import initRules from "./gen/rules/mtrack_rules";
import rulesUrl from "./gen/rules/mtrack_rules_bg.wasm?url";
// Importing the theme module subscribes the document to the user's
// stored / system theme preference and applies `.nc--dark` to <html>
// before the app mounts, so we never flash light-on-dark.
import "./lib/theme";
import "./app.css";
import App from "./App.svelte";
import { mount } from "svelte";
import { connect } from "./lib/ws/stores";

// The websocket that feeds the stores: opened here, before the rules module
// has loaded, so the first messages are waiting when the app mounts.
connect();

async function start() {
  const target = document.getElementById("app")!;
  try {
    await Promise.all([waitLocale(), initRules({ module_or_path: rulesUrl })]);
  } catch (error) {
    // Without the rules module every position and meter on every page
    // would be a question with no answer: say so instead of a blank page.
    console.error("could not load the player's rules module", error);
    target.textContent =
      "Could not load the player's rules module. Reload the page; if this keeps happening, the UI build is incomplete.";
    return;
  }
  mount(App, { target });
}

start();
