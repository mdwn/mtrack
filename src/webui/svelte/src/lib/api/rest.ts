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

export async function uploadFile(path: string, file: File): Promise<Response> {
  return fetch(`${BASE}${path}`, {
    method: "PUT",
    body: file,
  });
}

export async function uploadFiles(
  path: string,
  files: File[],
): Promise<Response> {
  const form = new FormData();
  for (const f of files) {
    form.append("file", f, f.name);
  }
  return fetch(`${BASE}${path}`, {
    method: "POST",
    body: form,
  });
}

export async function del(path: string): Promise<Response> {
  return request("DELETE", path);
}

export async function putYaml(path: string, body: string): Promise<Response> {
  return fetch(`${BASE}${path}`, {
    method: "PUT",
    headers: { "Content-Type": "text/yaml" },
    body,
  });
}

export async function postYaml(path: string, body: string): Promise<Response> {
  return fetch(`${BASE}${path}`, {
    method: "POST",
    headers: { "Content-Type": "text/yaml" },
    body,
  });
}

export async function putText(path: string, body: string): Promise<Response> {
  return fetch(`${BASE}${path}`, {
    method: "PUT",
    headers: { "Content-Type": "text/plain" },
    body,
  });
}

export async function postText(path: string, body: string): Promise<Response> {
  return fetch(`${BASE}${path}`, {
    method: "POST",
    headers: { "Content-Type": "text/plain" },
    body,
  });
}
