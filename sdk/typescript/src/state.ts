import type { ConexError } from "./errors";
import type { StateSample } from "./generated/conex/capability";
import { validateSample } from "./s1-wire";

export type Sample<T> = Required<Pick<StateSample, "revision" | "sampledAtMs">> & {
  value: T;
  quality: "current" | "stale";
};
export interface StateObserver<T> {
  update(sample: Sample<T>): void;
  end(error: unknown): void;
}
export interface StateOpening<T> { snapshot: Sample<T>; dispose(): void }
/** Must capture snapshot and install observer in ONE serialized domain operation. */
export type StateProvider<I, T> = (argument: I, observer: StateObserver<T>, signal: AbortSignal) => StateOpening<T> | Promise<StateOpening<T>>;

/** Synchronous JS mutations and opening share the same run-to-completion boundary. */
export class AtomicState<T> {
  private sample: Sample<T>;
  private readonly observers = new Set<StateObserver<T>>();
  constructor(initial: Sample<T>) {
    validateSample("snapshot", initial);
    this.sample = structuredClone(initial);
  }
  read(): Sample<T> { return structuredClone(this.sample); }
  publish(sample: Sample<T>): void {
    validateSample("sample", sample);
    if (BigInt(sample.revision) <= BigInt(this.sample.revision)) throw new Error("revision must increase");
    this.sample = structuredClone(sample);
    for (const observer of [...this.observers]) observer.update(this.read());
  }
  readonly open = (_argument: unknown, observer: StateObserver<T>, signal: AbortSignal): StateOpening<T> => {
    if (signal.aborted) throw new Error("subscription stopped");
    this.observers.add(observer);
    const dispose = () => {
      this.observers.delete(observer);
      signal.removeEventListener("abort", dispose);
    };
    signal.addEventListener("abort", dispose, { once: true });
    return { snapshot: this.read(), dispose };
  };
}

export interface StateSubscription<T> {
  readonly subscriptionId: string;
  readonly snapshot: Promise<Sample<T>>;
  /** Resolves once, including local unsubscribe; retained values are stale afterwards. */
  readonly ended: Promise<ConexError>;
  unsubscribe(): Promise<{ removed: boolean }>;
}
