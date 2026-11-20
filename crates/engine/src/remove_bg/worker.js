// FigCraft background remover: a Cloudflare Worker that FigCraft installs in the user's own
// Cloudflare account (Preferences > AI tools > Remove background). FigCraft never runs this file;
// it only uploads it. It is JavaScript because Cloudflare Workers run JavaScript.
//
// POST /remove-background  body: the image's bytes, header x-access-key: the key FigCraft made
//                          -> the cut-out as a PNG the size of the image (transparent background)
// GET  /                   -> a line saying the remover is running
//
// The cut-out is Cloudflare Images' foreground segmentation (`segment: "foreground"`, BiRefNet).
// Bindings, set by FigCraft when it installs the Worker: IMAGES (Cloudflare Images) and
// ACCESS_KEY (a secret).
//
// Licensed MIT OR Apache-2.0, like the rest of FigCraft.

const MAX_BYTES = 20 * 1024 * 1024; // Cloudflare Images' input limit for a Worker.

// Without this check anyone who finds the address could spend the account's free images.
// Compared in constant time, so the key can't be guessed from how long a refusal takes.
function keysMatch(given, expected) {
  const enc = new TextEncoder();
  const a = enc.encode(given);
  const b = enc.encode(expected);
  if (a.byteLength !== b.byteLength) return false;
  return crypto.subtle.timingSafeEqual(a, b);
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (request.method === "GET" && url.pathname === "/") {
      return new Response("FigCraft background remover is running.\n", { headers: { "content-type": "text/plain; charset=utf-8" } });
    }

    if (request.method !== "POST" || url.pathname !== "/remove-background") {
      return new Response("Not found", { status: 404 });
    }

    const key = request.headers.get("x-access-key") ?? "";
    if (!env.ACCESS_KEY || !keysMatch(key, env.ACCESS_KEY)) {
      return new Response("Wrong or missing access key", { status: 401 });
    }
    const length = Number(request.headers.get("content-length") ?? "0");
    if (!request.body || length === 0) return new Response("No image sent", { status: 400 });
    if (length > MAX_BYTES) return new Response("Image is over 20 MB", { status: 413 });

    try {
      const result = await env.IMAGES.input(request.body)
        .transform({ segment: "foreground" })
        .output({ format: "image/png" });
      return result.response({ headers: { "cache-control": "no-store" } });
    } catch (err) {
      return new Response("Background removal failed: " + (err?.message ?? String(err)), { status: 502 });
    }
  },
};
