/**
 * Shareable example links.
 *
 * A link carries only the example id, so it always resolves against the
 * examples bundled with the deployed Playground: `https://host/?example=trait`.
 */

export const EXAMPLE_PARAM = "example";

/** Reads the example id from a `location.search` string, if present. */
export function readSharedExampleId(search) {
  if (typeof search !== "string" || search.length === 0) {
    return null;
  }

  const value = new URLSearchParams(search).get(EXAMPLE_PARAM);
  if (value === null) {
    return null;
  }

  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

/** Builds the shareable link for an example id, based on the current href. */
export function buildExampleLink(href, id) {
  const url = new URL(href);
  url.hash = "";
  url.searchParams.set(EXAMPLE_PARAM, id);
  return url.toString();
}

/** Returns the href without the example parameter. */
export function stripExampleParam(href) {
  const url = new URL(href);
  if (!url.searchParams.has(EXAMPLE_PARAM)) {
    return href;
  }

  url.searchParams.delete(EXAMPLE_PARAM);
  return url.toString();
}
