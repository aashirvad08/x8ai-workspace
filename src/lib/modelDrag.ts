/**
 * A model dragged from the catalog onto the terminal. The data is only the
 * provider and model ids; what opens, and whether it may, is decided as for any
 * launch (the agent that can use it, trust, approval).
 */
export const MODEL_DRAG_TYPE = "application/x-x8ai-model";

export interface DraggedModel {
  readonly provider: string;
  readonly model: string;
  /** Shown while dragging over the terminal. */
  readonly name: string;
}

type Transfer = Pick<DataTransfer, "setData" | "getData" | "types">;

export function setModelDrag(transfer: Transfer, dragged: DraggedModel): void {
  transfer.setData(MODEL_DRAG_TYPE, JSON.stringify(dragged));
  transfer.setData("text/plain", dragged.model);
}

/** Whether a drag carries a model (its data is readable only on drop). */
export function carriesModel(transfer: Pick<DataTransfer, "types">): boolean {
  return [...transfer.types].includes(MODEL_DRAG_TYPE);
}

/** The dropped model, or `null` for anything else or anything malformed. */
export function readModelDrag(transfer: Transfer): DraggedModel | null {
  try {
    const value = JSON.parse(transfer.getData(MODEL_DRAG_TYPE)) as Partial<DraggedModel>;
    const text = (v: unknown, max: number) => typeof v === "string" && v.length > 0 && v.length <= max;
    if (!text(value.provider, 64) || !text(value.model, 300) || !text(value.name, 300)) return null;
    return { provider: value.provider!, model: value.model!, name: value.name! };
  } catch {
    return null;
  }
}
