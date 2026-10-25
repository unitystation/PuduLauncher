import { useState } from "react";
import {
    Accordion,
    AccordionDetails,
    AccordionGroup,
    AccordionSummary,
    Alert,
    Box,
    Button,
    Modal,
    ModalDialog,
    Stack,
    Typography,
} from "@mui/joy";

export interface FatalErrorModalItem {
    source: string;
    userMessage: string;
    code?: string | null;
    correlationId?: string | null;
    timestamp: string;
}

export interface FatalErrorModalProps {
    error: FatalErrorModalItem | null;
    trace: string;
    copyFeedback?: string | null;
    onCopyTrace: () => void | Promise<void>;
    onDismiss: () => void;
    onSeeLogs?: () => void;
    onDisableSandbox?: () => void | Promise<void>;
}

export default function FatalErrorModal(props: FatalErrorModalProps) {
    const { error, trace, copyFeedback, onCopyTrace, onDismiss, onSeeLogs, onDisableSandbox } = props;
    const [confirmDisableSandbox, setConfirmDisableSandbox] = useState(false);

    return (
        <>
            <Modal open={error !== null}>
                <ModalDialog layout="fullscreen" sx={{ p: 3 }}>
                    <Stack spacing={2} sx={{ height: "100%" }}>
                        <Typography level="h2">
                            {error?.code === "SANDBOX_LAUNCH_FAILED" ? "Game launch failed" : "Fatal error"}
                        </Typography>

                        <Alert color="danger" variant="soft">
                            {error?.userMessage}
                        </Alert>

                        <AccordionGroup>
                            <Accordion>
                                <AccordionSummary>Show diagnostic details and logs</AccordionSummary>
                                <AccordionDetails>
                                    <Box
                                        component="pre"
                                        sx={{
                                            p: 2,
                                            m: 0,
                                            borderRadius: "sm",
                                            bgcolor: "background.level1",
                                            overflow: "auto",
                                            maxHeight: "45vh",
                                            fontSize: 12,
                                        }}
                                    >
                                        {trace}
                                    </Box>
                                </AccordionDetails>
                            </Accordion>
                        </AccordionGroup>

                        <Stack direction="row" spacing={1}>
                            {error?.code === "SANDBOX_LAUNCH_FAILED" && onDisableSandbox && (
                                <Button onClick={() => setConfirmDisableSandbox(true)}>
                                    Disable sandbox for this session
                                </Button>
                            )}
                            <Button onClick={() => void onCopyTrace()}>Copy trace</Button>
                            {onSeeLogs && (
                                <Button variant="outlined" onClick={onSeeLogs}>
                                    See logs
                                </Button>
                            )}
                            <Button variant="outlined" onClick={onDismiss}>
                                Dismiss
                            </Button>
                            {copyFeedback && (
                                <Typography level="body-sm" sx={{ alignSelf: "center" }}>
                                    {copyFeedback}
                                </Typography>
                            )}
                        </Stack>
                    </Stack>
                </ModalDialog>
            </Modal>

            <Modal open={confirmDisableSandbox} onClose={() => setConfirmDisableSandbox(false)}>
                <ModalDialog>
                    <Stack spacing={2}>
                        <Typography level="h3">Disable sandbox for this session?</Typography>
                        <Alert color="warning" variant="soft">
                            Sandboxing adds a layer of security when running the game. Only disable it if you are sure
                            you can trust the build you are running.
                        </Alert>
                        <Stack direction="row" spacing={1} justifyContent="flex-end">
                            <Button variant="outlined" onClick={() => setConfirmDisableSandbox(false)}>
                                Keep sandbox enabled
                            </Button>
                            <Button
                                color="danger"
                                onClick={() => {
                                    setConfirmDisableSandbox(false);
                                    void onDisableSandbox?.();
                                }}
                            >
                                Disable for this session
                            </Button>
                        </Stack>
                    </Stack>
                </ModalDialog>
            </Modal>
        </>
    );
}
