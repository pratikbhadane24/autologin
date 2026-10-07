import { useCallback, useSyncExternalStore } from "react";

/** Phones and narrow windows; keep in step with the `max-width: 640px` media queries in the CSS. */
export const PHONE_QUERY = "(max-width: 640px)";

function matches(query: string): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia(query).matches;
}

/** Whether a CSS media query matches now; re-renders when that changes (rotation, window resize). */
export function useMediaQuery(query: string): boolean {
  const subscribe = useCallback(
    (onChange: () => void) => {
      if (typeof window.matchMedia !== "function") return () => undefined;
      const list = window.matchMedia(query);
      list.addEventListener("change", onChange);
      return () => list.removeEventListener("change", onChange);
    },
    [query],
  );
  return useSyncExternalStore(subscribe, () => matches(query));
}
