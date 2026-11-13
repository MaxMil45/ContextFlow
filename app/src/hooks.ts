import { useCallback, useEffect, useRef, useState } from "react";

import { toCommandError, type CommandError } from "./ipc/client";

export type AsyncState<T> = {
  data: T | undefined;
  error: CommandError | undefined;
  loading: boolean;
  reload: () => void;
};

/**
 * Load data from a command and reload on demand. A response that arrives after
 * a newer request was started is dropped, so fast switching never shows stale data.
 */
export function useCommand<T>(load: () => Promise<T>, deps: unknown[]): AsyncState<T> {
  const [data, setData] = useState<T>();
  const [error, setError] = useState<CommandError>();
  const [loading, setLoading] = useState(true);
  const [tick, setTick] = useState(0);
  const seq = useRef(0);

  useEffect(() => {
    const id = ++seq.current;
    setLoading(true);
    load()
      .then((d) => {
        if (id !== seq.current) return;
        setData(d);
        setError(undefined);
      })
      .catch((e) => {
        if (id !== seq.current) return;
        setError(toCommandError(e));
      })
      .finally(() => {
        if (id === seq.current) setLoading(false);
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, tick]);

  const reload = useCallback(() => setTick((t) => t + 1), []);
  return { data, error, loading, reload };
}
