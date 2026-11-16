/**
 * `@kumikit/ui-sdk` 0.1: what a UI plugin sees (`docs/spec/07-ui-plugins.md`).
 *
 * Framework-agnostic on purpose (ADR 0004): contributions are `mount(el, props) →
 * unmount` and plain data. Nothing here depends on Svelte or any other UI library.
 */
import type {
  Approval,
  EventNotification,
  Methods,
  ModelInfo,
  Resolution,
  Thread,
  ThreadStatus,
  TurnOutcome,
} from '@kumikit/protocol';

import type { Slot, TokenName } from './generated';

export type { Slot, TokenName, TokenType } from './generated';
export { SDK_VERSION, SLOTS, TOKENS } from './generated';
export { isComposing, tokenVar } from './helpers';

/** Undoes a registration or a mount. */
export type Dispose = () => void;

/** What `mount` returns: a function that removes what it added, or nothing. */
export type Unmount = Dispose | void;

/** Something the UI kernel mounts into an element it owns. */
export interface Mountable<P> {
  /** Draws into `el`. Must not touch anything outside it. */
  mount(el: HTMLElement, props: P): Unmount;
}

/** Where a thread lives: `<connection>/<workspace>/<thread>` (06-client, Addressing). */
export interface ThreadAddress {
  connection: string;
  workspace: string;
  thread: string;
}

// ---- the folded thread view ----

/** One entry in a turn's transcript, folded from thread events. */
export type TranscriptItem =
  | { kind: 'message'; text: string; done: boolean }
  | { kind: 'reasoning'; text: string }
  | {
      kind: 'tool';
      call: string;
      name: string;
      input: unknown;
      output: string;
      ok?: boolean;
      result?: unknown;
    }
  | { kind: 'file.changed'; path: string; diff?: string }
  | { kind: 'approval'; approval: Approval; resolution?: Resolution }
  | { kind: 'plugin.event'; plugin: string; event: string; data: unknown };

/** The kind strings of transcript items; `plugin.event` items also match
 * `plugin.event:<plugin>/<kind>`. */
export type TranscriptKind = TranscriptItem['kind'];

/** One turn: the user's input and what followed. */
export interface TurnView {
  id: string;
  input: string;
  started: string;
  ended?: string;
  outcome?: TurnOutcome;
  error?: string;
  items: TranscriptItem[];
  usage?: { inputTokens: number; outputTokens: number; cost?: number };
}

/** A thread as the UI kernel folds it from its event log. Plain data; replaced, not
 * mutated, on every change. */
export interface ThreadView {
  address: ThreadAddress;
  /** The thread record, once known. */
  thread?: Thread;
  /** Derived from the log, as the core derives it. */
  status: ThreadStatus;
  /** The last `seq` folded in. */
  seq: number;
  turns: TurnView[];
  pendingApprovals: Approval[];
  /** Messages sent while a turn runs, in order; they start when it ends. */
  queued: string[];
  /** Whether the connection is live (false while reconnecting). */
  live: boolean;
}

// ---- slots ----

/** Props of a `transcript.renderer` mount. */
export interface RendererProps {
  item: TranscriptItem;
  turn: TurnView;
  view: ThreadView;
  /** Runs a kernel or plugin command (for example to answer an approval). */
  run(command: string, args?: unknown): Promise<unknown>;
}

/** Props given to mounts that belong to a thread. */
export interface ThreadProps {
  address: ThreadAddress;
  /** The connection the thread is on. */
  connection: Connection;
}

/** Props of a `composer.accessory` mount. */
export interface ComposerProps extends ThreadProps {
  /** Inserts text at the composer's cursor. */
  insert(text: string): void;
}

/** What each slot accepts. */
export interface SlotContributions {
  'sidebar.section': { id: string; title: string } & Mountable<{ connection: Connection }>;
  'inspector.tab': { id: string; title: string } & Mountable<Partial<ThreadProps>>;
  'statusbar.item': { id: string } & Mountable<Record<string, never>>;
  'composer.accessory': { id: string } & Mountable<ComposerProps>;
  'thread.header': { id: string } & Mountable<ThreadProps>;
  'thread.view': { id: string } & Mountable<ThreadProps>;
  /** `kind` is a transcript item kind, or `plugin.event:<plugin>/<kind>`. */
  'transcript.renderer': { kind: string } & Mountable<RendererProps>;
  'settings.page': { id: string; title: string; order?: number } & Mountable<
    Record<string, never>
  >;
  view: { id: string; title: string } & Mountable<{ args?: unknown }>;
  command: CommandSpec;
}

/** A palette command. */
export interface CommandSpec {
  id: string;
  title: string;
  /** For example `Mod+K`; `Mod` is Ctrl, or Cmd on macOS. */
  keybinding?: string;
  /** `thread.open`, `connection.ready`; joined with `&&`. */
  when?: string;
}

// ---- the context ----

/** A connection to one core, limited to what the plugin declared. */
export interface Connection {
  /** The connection's id (`local` in M2). */
  readonly id: string;
  /** Whether it is connected now. */
  readonly ready: boolean;
  call<M extends keyof Methods>(
    method: M,
    params: Methods[M]['params'],
  ): Promise<Methods[M]['result']>;
  /** Follows a topic; events arrive after `after`. The UI kernel holds the one wire
   * subscription per topic and fans it out. */
  subscribe(topic: string, after: number, fn: (event: EventNotification) => void): Dispose;
  /** The models of a provider (cached by the UI kernel). */
  models(provider: string): Promise<ModelInfo[]>;
}

/** A plugin's view of the UI kernel. */
export interface PluginContext {
  /** This plugin's id. */
  readonly id: string;
  slots: {
    register<S extends Slot>(slot: S, contribution: SlotContributions[S]): Dispose;
  };
  commands: {
    register(spec: CommandSpec, handler: (args?: unknown) => unknown): Dispose;
    run(id: string, args?: unknown): Promise<unknown>;
  };
  /** The active tab's connection, if any. */
  readonly connection: Connection | undefined;
  threads: {
    /** Calls `fn` with the folded view now and after every change. */
    watch(address: ThreadAddress, fn: (view: ThreadView) => void): Dispose;
  };
  theme: {
    /** Resolved token values of the active theme. */
    readonly tokens: Readonly<Partial<Record<TokenName, string>>>;
    readonly appearance: 'light' | 'dark';
    onChange(fn: () => void): Dispose;
  };
  /** Small per-plugin storage on this client (strings only). */
  storage: {
    get(key: string): string | undefined;
    set(key: string, value: string): void;
    delete(key: string): void;
  };
  /** Shows a short message. */
  notify(message: string, tone?: 'info' | 'success' | 'warning' | 'danger'): void;
  ui: {
    /** Renders markdown into `el` with the UI kernel's renderer. Raw HTML is shown as
     * text; links are not followed. */
    markdown(el: HTMLElement, text: string): void;
  };
}

/** A UI plugin module's default export. */
export interface Plugin {
  activate(ctx: PluginContext): void | Promise<void>;
  deactivate?(): void | Promise<void>;
}
