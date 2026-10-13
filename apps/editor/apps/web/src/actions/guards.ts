import { ACTIONS } from "./definitions";
import type { TActionWithOptionalArgs } from "./types";

// VALIDATION PATCH: upstream persistence.ts imports this but it was never defined.
export function isActionWithOptionalArgs(
	value: string,
): value is TActionWithOptionalArgs {
	return Object.prototype.hasOwnProperty.call(ACTIONS, value);
}
