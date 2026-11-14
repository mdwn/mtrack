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

// Answering the web UI's gRPC-Web calls from a spec, by method name.
//
//   const rpc = await mockRpc(page);
//   rpc.unary(PlayerService.method.listPlaylists, { playlists: [...] });
//   rpc.unary("PlayerService/Seek", (req) => ({}));   // a function of the request
//   rpc.error(PlayerService.method.play, Code.FailedPrecondition, "Player is locked.");
//   await page.getByRole("button", { name: "Play" }).click();
//   expect(rpc.calls(PlayerService.method.play)).toHaveLength(1);
//
// It is what `page.route("**/api/...")` is for the REST calls, for the calls
// the app makes through the generated client (src/lib/grpc/client.ts): the
// answer is a response message in the generated type, which the helper
// encodes (./grpc-web.ts); a request is decoded to the generated type before
// the answer or the recording sees it. A method nothing answers falls through
// to the mock server, as do the streams (`WatchStage`): they are not
// answered from here, whatever is registered.
import type { Page } from "@playwright/test";
import {
  create,
  fromBinary,
  toBinary,
  type DescMessage,
  type DescMethod,
  type DescMethodUnary,
  type DescService,
  type MessageInitShape,
  type MessageShape,
} from "@bufbuild/protobuf";
import { Code, ConnectError } from "@connectrpc/connect";
import {
  ConfigService,
  LightingService,
  PlayerService,
} from "../../src/gen/player/v1/player_pb";
import { errorHeaders, requestMessage, unaryBody } from "./grpc-web";

/** The services a spec can name. Add each service here as the proto gains
 *  one: the route and the by-name lookup read the methods from these
 *  descriptors. */
const SERVICES: DescService[] = [PlayerService, ConfigService, LightingService];

/** How a call nothing answers is answered, when {@link RpcMockOptions.otherwise}
 *  is set.
 *  - `trailers-only`: an empty body, its status in the headers.
 *  - `framed`: a well-formed unary reply, an empty message frame then a
 *    trailers frame. A bare empty body makes the client's response parsing
 *    reject, which silently aborts call chains like pause's stop-then-seek:
 *    a spec that follows such a chain needs this one. */
export type GrpcReply = "trailers-only" | "framed";

export interface RpcMockOptions {
  /** Answers every unary call no `unary` or `error` answers with an empty
   *  success, instead of letting it fall through to the mock server. */
  otherwise?: GrpcReply;
  /** Called for each call the route takes, answered or not, before it is. */
  onCall?: (call: RecordedCall) => void;
}

/** One call the route took. `request` is undefined for a method the helper
 *  does not know (it cannot decode the body). */
export interface RecordedCall {
  /** The service's full name, `player.v1.PlayerService`. */
  service: string;
  /** The method's name, `ListPlaylists`. */
  method: string;
  request: unknown;
}

/** A method as a spec names it: its generated descriptor
 *  (`PlayerService.method.listPlaylists`), which types the answer, or
 *  `Service/Method` by name (`PlayerService/ListPlaylists`, or with the
 *  service's package, `player.v1.PlayerService/ListPlaylists`). */
export type MethodRef<
  I extends DescMessage = DescMessage,
  O extends DescMessage = DescMessage,
> = DescMethodUnary<I, O> | string;

type Answer = (request: unknown) => unknown;

/** Looks a method up by name in {@link SERVICES}. */
function findMethod(ref: string): DescMethod {
  const [service, name, ...rest] = ref.split("/");
  const found = SERVICES.filter(
    (s) => s.typeName === service || s.name === service,
  )
    .flatMap((s) => s.methods)
    .find((m) => m.name === name || m.localName === name);
  if (!found || !name || rest.length > 0) {
    throw new Error(
      `no method "${ref}": name it as Service/Method, of ${SERVICES.map((s) => s.typeName).join(", ")}`,
    );
  }
  return found;
}

function resolve(ref: MethodRef): DescMethodUnary {
  const method: DescMethod = typeof ref === "string" ? findMethod(ref) : ref;
  if (method.methodKind !== "unary") {
    throw new Error(
      `${method.parent.typeName}/${method.name} is a ${method.methodKind} method; only unary methods are answered from a spec`,
    );
  }
  return method as DescMethodUnary;
}

const keyOf = (method: DescMethod) =>
  `${method.parent.typeName}/${method.name}`;

/** The answers a page's calls get, and the calls it made. */
export class RpcMock {
  private answers = new Map<string, Answer>();
  private recorded: RecordedCall[] = [];

  /** @internal Called by the route. */
  constructor(readonly options: RpcMockOptions) {}

  /** Answers `method` with a message: `answer` is the response, or a function
   *  of the decoded request that returns it. A function may throw a
   *  `ConnectError` to fail the call instead. Answering a method again
   *  replaces the earlier answer. */
  unary<I extends DescMessage, O extends DescMessage>(
    method: DescMethodUnary<I, O>,
    answer:
      | MessageInitShape<O>
      | ((request: MessageShape<I>) => MessageInitShape<O>),
  ): void;
  unary(
    method: string,
    answer:
      | Record<string, unknown>
      | ((request: unknown) => Record<string, unknown>),
  ): void;
  unary(method: MethodRef, answer: unknown): void {
    const desc = resolve(method);
    this.answers.set(keyOf(desc), (request) => {
      const init =
        typeof answer === "function"
          ? (answer as (r: unknown) => unknown)(request)
          : answer;
      return unaryBody(
        toBinary(desc.output, create(desc.output, init as never)),
      );
    });
  }

  /** Fails `method` with a gRPC status `code` (a `Code` of
   *  `@connectrpc/connect`: `Code.NotFound`, `Code.FailedPrecondition` for a
   *  locked player, `Code.Aborted` for a changed revision) and `message`,
   *  with the status's `metadata` (`x-current-version`) if given. */
  error(
    method: MethodRef,
    code: Code,
    message: string,
    metadata?: HeadersInit,
  ): void {
    const desc = resolve(method);
    this.answers.set(
      keyOf(desc),
      () => new ConnectError(message, code, metadata),
    );
  }

  /** The decoded requests the page has sent `method`, in order. */
  calls<I extends DescMessage, O extends DescMessage>(
    method: DescMethodUnary<I, O>,
  ): MessageShape<I>[];
  calls(method: string): unknown[];
  calls(method: MethodRef): unknown[] {
    const key = keyOf(resolve(method));
    return this.recorded
      .filter((c) => `${c.service}/${c.method}` === key)
      .map((c) => c.request);
  }

  /** Every call the route took, in order. */
  get allCalls(): readonly RecordedCall[] {
    return this.recorded;
  }

  /** @internal What the route answers `call` with: the response body, or
   *  `undefined` to let the request fall through. */
  respond(
    service: string,
    name: string,
    body: Uint8Array,
  ): { body: Uint8Array; headers?: Record<string, string> } | undefined {
    const method = SERVICES.find((s) => s.typeName === service)?.methods.find(
      (m) => m.name === name,
    );
    // The streams are the mock server's.
    if (method && method.methodKind !== "unary") return undefined;
    let request: unknown;
    if (method) {
      request = fromBinary(method.input, requestMessage(body));
    }
    const call: RecordedCall = { service, method: name, request };
    this.recorded.push(call);
    this.options.onCall?.(call);

    const answer = this.answers.get(`${service}/${name}`);
    if (answer) {
      let answered: unknown;
      try {
        answered = answer(request);
      } catch (e) {
        answered = ConnectError.from(e);
      }
      if (answered instanceof ConnectError) {
        return {
          body: new Uint8Array(),
          headers: errorHeaders(
            answered.code,
            answered.rawMessage,
            answered.metadata,
          ),
        };
      }
      return { body: answered as Uint8Array };
    }
    switch (this.options.otherwise) {
      case "framed":
        return { body: unaryBody(new Uint8Array()) };
      case "trailers-only":
        return {
          body: new Uint8Array(),
          headers: {
            "content-type": "application/grpc-web+proto",
            "grpc-status": "0",
          },
        };
      default:
        return undefined;
    }
  }
}

/** Routes the page's gRPC-Web calls (`**\/player.v1.*Service/*`) to the
 *  returned {@link RpcMock}. Register answers on it before the page makes
 *  the calls; a second `mockRpc` on the same page is consulted first, and
 *  what it does not answer falls to the first. */
export async function mockRpc(
  page: Page,
  options: RpcMockOptions = {},
): Promise<RpcMock> {
  const mock = new RpcMock(options);
  await page.route("**/player.v1.*Service/*", async (route) => {
    const request = route.request();
    const [service, method] = new URL(request.url()).pathname
      .split("/")
      .slice(-2);
    const answer = mock.respond(
      service,
      method,
      new Uint8Array(request.postDataBuffer() ?? []),
    );
    if (!answer) {
      await route.fallback();
      return;
    }
    await route.fulfill({
      status: 200,
      headers: answer.headers ?? {
        "content-type": "application/grpc-web+proto",
      },
      body: Buffer.from(answer.body),
    });
  });
  return mock;
}
