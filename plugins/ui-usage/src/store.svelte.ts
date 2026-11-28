/**
 * The usage data of one connection, shared by every meter and dialog on it: the core's
 * providers, their latest plan limits, and a counter that moves when a turn adds usage.
 * One `usage` topic subscription per connection, held while any view uses it.
 */
import type { ProviderInfo, ProviderLimits } from '@kumikit/protocol';
import type { Connection } from '@kumikit/ui-sdk';

export class UsageStore {
  providers = $state<ProviderInfo[]>([]);
  limits = $state<Record<string, ProviderLimits>>({});
  /** Moves when a turn on this core adds usage, so open views refetch statistics. */
  version = $state(0);
  /** Providers whose refresh is running. */
  refreshing = $state<Record<string, boolean>>({});
  readonly connection: Connection;
  #stop: (() => void) | undefined;
  #loading: Promise<void> | undefined;

  constructor(connection: Connection) {
    this.connection = connection;
  }

  start(): void {
    this.#stop = this.connection.subscribe('usage', 0, (e) => {
      if (e.kind === 'usage.limits') {
        this.limits = { ...this.limits, [e.data.provider]: e.data };
      } else if (e.kind === 'usage.recorded') {
        this.version += 1;
      }
    });
  }

  stop(): void {
    this.#stop?.();
    this.#stop = undefined;
  }

  /** Reads the providers and their limits; concurrent calls share one read. */
  load(): Promise<void> {
    this.#loading ??= (async () => {
      try {
        const [list, limits] = await Promise.all([
          this.connection.call('provider.list', {}),
          this.connection.call('usage.limits', {}),
        ]);
        this.providers = list.providers;
        // A snapshot that arrived on the topic while this read ran may be newer.
        this.limits = Object.fromEntries(limits.providers.map((p) => [p.provider, newer(p, this.limits[p.provider])]));
      } finally {
        this.#loading = undefined;
      }
    })();
    return this.#loading;
  }

  /** Asks the provider to read its plan limits now. */
  async refresh(provider: string): Promise<void> {
    this.refreshing = { ...this.refreshing, [provider]: true };
    try {
      const got = await this.connection.call('usage.refresh', { provider });
      this.limits = { ...this.limits, [provider]: got };
    } finally {
      const { [provider]: _, ...rest } = this.refreshing;
      this.refreshing = rest;
    }
  }
}

/** The later of two snapshots of one provider, by when they were taken. */
export function newer(a: ProviderLimits, b: ProviderLimits | undefined): ProviderLimits {
  if (!b?.observed) return a;
  if (!a.observed) return b;
  return Date.parse(b.observed) > Date.parse(a.observed) ? b : a;
}

const stores = new Map<string, { store: UsageStore; users: number }>();

/** The store of `connection`, started on first use; call the returned function when
 * done with it. */
export function acquire(connection: Connection): [UsageStore, () => void] {
  let entry = stores.get(connection.id);
  if (!entry) {
    const store = new UsageStore(connection);
    store.start();
    entry = { store, users: 0 };
    stores.set(connection.id, entry);
  }
  entry.users += 1;
  const held = entry;
  let released = false;
  return [
    held.store,
    () => {
      if (released) return;
      released = true;
      held.users -= 1;
      if (held.users === 0) {
        held.store.stop();
        stores.delete(connection.id);
      }
    },
  ];
}
