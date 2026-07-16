import { beforeEach, describe, expect, it } from "vitest";

import { attemptStateRehydrate } from "../engineRecovery";
import { useGameStore } from "../../stores/gameStore";
import { buildEngineAdapterMock } from "../../test/factories/engineAdapterFactory";
import { buildGameState } from "../../test/factories/gameStateFactory";

describe("attemptStateRehydrate", () => {
  beforeEach(() => {
    useGameStore.getState().reset();
  });

  it("restores the local authoritative state rather than the viewer projection", async () => {
    const viewerState = buildGameState({ turn_number: 1 });
    const authoritativeState = buildGameState({ turn_number: 11 });
    const adapter = buildEngineAdapterMock(viewerState);
    useGameStore.setState({
      adapter,
      gameState: viewerState,
      authoritativeGameState: authoritativeState,
      gameMode: "local",
    });

    await expect(attemptStateRehydrate()).resolves.toBe(true);
    expect(adapter.restoreState).toHaveBeenCalledWith(authoritativeState);
  });
});
