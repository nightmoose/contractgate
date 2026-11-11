// Resolve hook for dashboard unit tests. Node's type stripper does not add
// file extensions, and it does not read tsconfig paths.

const EXT = [".ts", ".tsx", ".js", ".mjs"];

export async function resolve(specifier, context, nextResolve) {
  let rewritten = specifier;
  if (specifier.startsWith("@/")) {
    rewritten = new URL("../" + specifier.slice(2), import.meta.url).href;
  }

  try {
    return await nextResolve(rewritten, context);
  } catch (err) {
    if (!err || err.code !== "ERR_MODULE_NOT_FOUND") throw err;
    const relative =
      rewritten.startsWith(".") || rewritten.startsWith("/") || rewritten.startsWith("file:");
    const candidates = relative ? EXT.map((ext) => rewritten + ext) : [rewritten + ".js"];

    let last = err;
    for (const candidate of candidates) {
      try {
        return await nextResolve(candidate, context);
      } catch (next) {
        last = next;
        if (!next || next.code !== "ERR_MODULE_NOT_FOUND") throw next;
      }
    }
    throw last;
  }
}
