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

const BASE = "/api";

async function request(
  method: string,
  path: string,
  body?: string,
): Promise<Response> {
  const opts: RequestInit = { method };
  if (body !== undefined) {
    opts.headers = { "Content-Type": "application/json" };
    opts.body = body;
  }
  return fetch(`${BASE}${path}`, opts);
}

export async function get(path: string): Promise<Response> {
  return request("GET", path);
}

export async function put(path: string, body: string): Promise<Response> {
  return request("PUT", path, body);
}

export async function post(path: string, body?: string): Promise<Response> {
  return request("POST", path, body);
}
