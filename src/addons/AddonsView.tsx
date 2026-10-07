import { memo, useEffect } from "react";

import type { AddonList } from "../contracts/generated/AddonList";
import type { AddonStatus } from "../contracts/generated/AddonStatus";
import { useStore } from "../lib/useStore";
import type { AddonActions } from "./actions";
import { type Addons, GROUPS, offReason } from "./addons";

interface Props {
  addons: Addons;
  actions: AddonActions;
}

/** Tools for the open space's terminals (⇧⌘X): a prompt, highlighting, an editor setup, a font. */
export const AddonsView = memo(function AddonsView({ addons, actions }: Props) {
  const { list, loading, error, busy } = useStore(addons);

  useEffect(() => {
    if (addons.get().list === null) void addons.load();
  }, [addons]);

  return (
    <div className="agents addons">
      <header className="explorer-header">
        <span className="explorer-title">Add-ons</span>
        <button type="button" className="icon-button" title="Look again at what is installed" aria-label="Refresh add-ons" onClick={() => actions.refreshAddons()}>
          ↻
        </button>
      </header>
      {list && (
        <p className="agents-hint" title={list.space.root ?? "The workspace with no folder open"}>
          For {list.space.name} <span className="addons-id">{list.space.id}</span>
          {list.space.addons.length > 0 && (
            <>
              {" · "}
              <button type="button" className="link-button" title="Give these add-ons to another space" onClick={() => actions.shareAddons()}>
                Share
              </button>
            </>
          )}
        </p>
      )}
      {error && (
        <p className="agents-note agents-error" role="alert">
          {error}
        </p>
      )}
      {list && !list.homebrew && (
        <p className="agents-note agents-note-warning">
          Add-ons install with Homebrew, which is not on this Mac. Install it from brew.sh, then refresh.
        </p>
      )}
      {list && !list.shellSupported && (
        <p className="agents-hint">Shell add-ons need zsh; your shell is {list.shell}.</p>
      )}
      {list === null && loading && <p className="agents-note">Looking at what is installed…</p>}
      {list &&
        GROUPS.map(({ group, label }) => {
          const items = list.addons.filter((a) => a.group === group);
          if (items.length === 0) return null;
          return (
            <section key={group} aria-label={label}>
              <h2 className="recent-title agents-heading">{label}</h2>
              <ul className="agents-list addons-list">
                {items.map((addon) => (
                  <AddonRow key={addon.id} addon={addon} list={list} busy={busy.includes(addon.id)} actions={actions} />
                ))}
              </ul>
            </section>
          );
        })}
    </div>
  );
});

function AddonRow({ addon, list, busy, actions }: { addon: AddonStatus; list: AddonList; busy: boolean; actions: AddonActions }) {
  const off = offReason(addon, list);
  const reach =
    addon.reach === "mac"
      ? "A program for your whole Mac: once installed, it works in every terminal."
      : `On only in the terminals of the spaces it is added to.`;
  return (
    <li className={addon.added ? "agent addon addon-added" : "agent addon"} aria-label={addon.name}>
      <div className="agent-heading">
        <span className="agent-name" title={reach}>
          {addon.name}
        </span>
        {busy ? (
          <span className="agent-status">Working…</span>
        ) : addon.added ? (
          <button
            type="button"
            className={addon.active ? "addon-toggle addon-on" : "addon-toggle addon-off"}
            title={`Remove from ${list.space.name}`}
            aria-label={`Remove ${addon.name} from ${list.space.name}`}
            onClick={() => actions.removeAddon(addon.id)}
          >
            <span className="addon-toggle-label">{addon.active ? "On" : "Off"}</span>
          </button>
        ) : (
          <button
            type="button"
            className="addon-add"
            title={addon.installed ? `Add to ${list.space.name}` : `Installs with: ${addon.install.join("; ")}`}
            aria-label={`Add ${addon.name}`}
            disabled={!addon.installed && !list.homebrew && addon.install.some((c) => c.startsWith("brew"))}
            onClick={() => actions.addAddon(addon.id)}
          >
            {addon.installed ? "Add" : "Install"}
          </button>
        )}
      </div>
      <p className="agent-hint addon-description" title={addon.description}>
        {addon.description}
      </p>
      {addon.usage && addon.added && (
        <p className="agent-hint">
          <code>{addon.usage}</code>
        </p>
      )}
      {off && <p className="agent-hint addon-reason">{off}</p>}
      {addon.inYourShell && (
        <p className="agent-hint" title="A line in your own ~/.zshrc (or another zsh startup file) turns it on.">
          Also on everywhere, by your own ~/.zshrc.
        </p>
      )}
    </li>
  );
}
