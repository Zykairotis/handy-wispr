import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import type {
  TokenStatus,
  TranscriptionProvider as Provider,
} from "@/bindings";
import { Alert } from "../ui/Alert";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";

interface TranscriptionProviderProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const TranscriptionProvider: React.FC<TranscriptionProviderProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const [confirmOpen, setConfirmOpen] = useState(false);
    const [tokenStatus, setTokenStatus] = useState<TokenStatus | null>(null);

    const provider = (getSetting("transcription_provider") ||
      "local") as Provider;
    const hasConsent = getSetting("wispr_consent") === true;

    useEffect(() => {
      if (provider !== "wispr") return;
      commands.getWisprTokenStatus().then(setTokenStatus);
    }, [provider]);

    const options = [
      {
        value: "local",
        label: t("settings.general.transcriptionProvider.options.local"),
      },
      {
        value: "wispr",
        label: t("settings.general.transcriptionProvider.options.wispr"),
      },
    ];

    const handleSelect = (value: string) => {
      // Uploading audio needs an explicit yes the first time.
      if (value === "wispr" && !hasConsent) {
        setConfirmOpen(true);
        return;
      }
      updateSetting("transcription_provider", value as Provider);
    };

    const handleConfirm = async () => {
      setConfirmOpen(false);
      await updateSetting("wispr_consent", true);
      await updateSetting("transcription_provider", "wispr");
    };

    return (
      <>
        <SettingContainer
          title={t("settings.general.transcriptionProvider.title")}
          description={t("settings.general.transcriptionProvider.description")}
          descriptionMode={descriptionMode}
          grouped={grouped}
        >
          <Dropdown
            options={options}
            selectedValue={provider}
            onSelect={handleSelect}
            disabled={isUpdating("transcription_provider")}
          />
        </SettingContainer>

        {provider === "wispr" && tokenStatus && tokenStatus !== "ok" && (
          <Alert variant="warning" contained>
            {t(`settings.general.transcriptionProvider.token.${tokenStatus}`)}
          </Alert>
        )}

        <Dialog
          open={confirmOpen}
          onOpenChange={setConfirmOpen}
          title={t("settings.general.transcriptionProvider.consent.title")}
          description={t("settings.general.transcriptionProvider.consent.body")}
          closeLabel={t("common.close")}
          footer={
            <div className="flex justify-end gap-2">
              <Button variant="secondary" onClick={() => setConfirmOpen(false)}>
                {t("settings.general.transcriptionProvider.consent.cancel")}
              </Button>
              <Button onClick={handleConfirm}>
                {t("settings.general.transcriptionProvider.consent.confirm")}
              </Button>
            </div>
          }
        >
          {null}
        </Dialog>
      </>
    );
  });
