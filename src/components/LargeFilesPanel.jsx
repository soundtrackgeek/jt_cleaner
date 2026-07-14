import { useEffect, useMemo, useState } from "react";
import {
  CheckmarkCircle24Regular,
  Delete24Regular,
  Dismiss24Regular,
  Document24Regular,
  ShieldCheckmark24Regular,
  Sparkle24Regular,
  Warning24Regular,
} from "@fluentui/react-icons";
import { formatBytes, formatDateTime } from "../lib/format.js";

const verdictLabels = {
  likelySafe: "Likely safe to delete",
  review: "Review before deleting",
  keep: "Keep this file",
};

function DeleteLargeFilesDialog({ files, busy, error, onCancel, onConfirm }) {
  const totalBytes = files.reduce((total, file) => total + file.sizeBytes, 0);
  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={busy ? undefined : onCancel}>
      <section className="confirm-dialog large-file-delete-dialog" role="dialog" aria-modal="true" aria-labelledby="large-file-delete-title" onMouseDown={(event) => event.stopPropagation()}>
        <button className="icon-button dialog-close" type="button" onClick={onCancel} disabled={busy} aria-label="Close deletion confirmation"><Dismiss24Regular /></button>
        <span className="dialog-icon is-danger"><Warning24Regular /></span>
        <h2 id="large-file-delete-title">Permanently delete {files.length} {files.length === 1 ? "file" : "files"}?</h2>
        <p>This removes {formatBytes(totalBytes)} immediately and does not use the Recycle Bin. Luna cannot undo this action.</p>
        <div className="large-file-delete-list">
          {files.slice(0, 4).map((file) => <span key={file.path}><Document24Regular /><strong>{file.name}</strong><small>{formatBytes(file.sizeBytes)}</small></span>)}
          {files.length > 4 && <em>and {files.length - 4} more</em>}
        </div>
        <div className="dialog-note is-danger"><ShieldCheckmark24Regular />Only files selected from the current scan will be accepted. Changed files stay untouched.</div>
        {error && <div className="inline-error"><Warning24Regular />{error}</div>}
        <div className="dialog-actions">
          <button className="secondary-button" type="button" onClick={onCancel} disabled={busy}>Keep files</button>
          <button className="danger-button" type="button" onClick={onConfirm} disabled={busy}><Delete24Regular />{busy ? "Deleting…" : "Delete permanently"}</button>
        </div>
      </section>
    </div>
  );
}

function FileAssessmentDialog({ file, envelope, busy, error, onClose }) {
  const assessment = envelope?.assessment;
  const verdict = assessment?.verdict || "review";
  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={busy ? undefined : onClose}>
      <section className="large-file-assessment-dialog" role="dialog" aria-modal="true" aria-labelledby="large-file-assessment-title" onMouseDown={(event) => event.stopPropagation()}>
        <button className="icon-button dialog-close" type="button" onClick={onClose} disabled={busy} aria-label="Close AI assessment"><Dismiss24Regular /></button>
        <header>
          <span><Sparkle24Regular /></span>
          <div><small>AI file opinion</small><strong id="large-file-assessment-title">{file.name}</strong><em>{formatBytes(file.sizeBytes)} · metadata only</em></div>
        </header>
        {busy ? (
          <div className="large-file-ai-loading" role="status"><span className="scan-spinner"><Sparkle24Regular /></span><strong>Luna is reviewing the metadata…</strong><p>No file contents are uploaded or opened.</p></div>
        ) : error ? (
          <div className="inline-error large-file-ai-error"><Warning24Regular />{error}</div>
        ) : assessment ? (
          <>
            <div className={`large-file-verdict verdict-${verdict}`}><span>{verdictLabels[verdict] || "Review before deleting"}</span><small>{assessment.confidence} confidence</small></div>
            <h2>{assessment.headline}</h2>
            <p className="large-file-assessment-summary">{assessment.explanation}</p>
            <div className="large-file-assessment-grid">
              <section><h3>Signals considered</h3><ul>{assessment.signals.map((signal) => <li key={signal}><CheckmarkCircle24Regular />{signal}</li>)}</ul></section>
              <section><h3>{verdict === "likelySafe" ? "Before deleting" : "Safer next steps"}</h3><ol>{assessment.suggestions.map((suggestion, index) => <li key={suggestion}><span>{index + 1}</span>{suggestion}</li>)}</ol></section>
            </div>
            <div className="large-file-ai-caution"><Warning24Regular />{assessment.caution}</div>
            <footer><span>{envelope.model} · generated {formatDateTime(envelope.generatedAt)}</span><strong>AI opinion, not a guarantee</strong></footer>
          </>
        ) : null}
      </section>
    </div>
  );
}

export function LargeFilesPanel({ result, onDeleteFiles, onAskAi }) {
  const [selectedPaths, setSelectedPaths] = useState([]);
  const [confirming, setConfirming] = useState(false);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState("");
  const [assessmentFile, setAssessmentFile] = useState(null);
  const [assessmentEnvelope, setAssessmentEnvelope] = useState(null);
  const [assessmentBusy, setAssessmentBusy] = useState(false);
  const [assessmentError, setAssessmentError] = useState("");

  useEffect(() => {
    setSelectedPaths([]);
    setConfirming(false);
    setDeleteError("");
    setAssessmentFile(null);
    setAssessmentEnvelope(null);
    setAssessmentError("");
  }, [result.scannedAt]);

  const selectedFiles = useMemo(() => {
    const selected = new Set(selectedPaths);
    return result.largeFiles.filter((file) => selected.has(file.path));
  }, [result.largeFiles, selectedPaths]);
  const selectedBytes = selectedFiles.reduce((total, file) => total + file.sizeBytes, 0);
  const allSelected = result.largeFiles.length > 0 && selectedFiles.length === result.largeFiles.length;

  function togglePath(path) {
    setSelectedPaths((current) => current.includes(path) ? current.filter((entry) => entry !== path) : [...current, path]);
    setDeleteError("");
  }

  async function deleteSelected() {
    if (!selectedFiles.length || deleteBusy) return;
    setDeleteBusy(true);
    setDeleteError("");
    try {
      const outcome = await onDeleteFiles(selectedFiles.map((file) => file.path));
      const deleted = new Set(outcome.deletedFiles.map((file) => file.path));
      setSelectedPaths((current) => current.filter((path) => !deleted.has(path)));
      if (outcome.failed.length) {
        setDeleteError(outcome.failed.join(" "));
      } else {
        setConfirming(false);
      }
    } catch (error) {
      setDeleteError(String(error));
    } finally {
      setDeleteBusy(false);
    }
  }

  async function askAi(file) {
    if (assessmentBusy) return;
    setAssessmentFile(file);
    setAssessmentEnvelope(null);
    setAssessmentError("");
    setAssessmentBusy(true);
    try {
      setAssessmentEnvelope(await onAskAi(file));
    } catch (error) {
      setAssessmentError(String(error));
    } finally {
      setAssessmentBusy(false);
    }
  }

  if (!result.largeFiles.length) {
    return <section className="empty-state-surface"><Document24Regular /><h2>No large files in this scan</h2><p>Try a broader folder or drive if you want to investigate elsewhere.</p></section>;
  }

  return (
    <>
      <section className="large-file-selection-bar" aria-live="polite">
        <div><strong>{selectedFiles.length ? `${selectedFiles.length} selected · ${formatBytes(selectedBytes)}` : "Select files to act on"}</strong><span>Nothing is selected automatically.</span></div>
        <button className="danger-button" type="button" disabled={!selectedFiles.length} onClick={() => setConfirming(true)}><Delete24Regular />Delete selected</button>
      </section>
      <section className="feature-surface large-file-table">
        <div className="data-table">
          <div className="data-header">
            <label className="large-file-check"><input type="checkbox" checked={allSelected} onChange={() => setSelectedPaths(allSelected ? [] : result.largeFiles.map((file) => file.path))} aria-label={allSelected ? "Clear all large-file selections" : "Select all large files"} /><span /></label>
            <span>File</span><span>Size</span><span>Activity</span><span>Opinion</span>
          </div>
          {result.largeFiles.map((file) => {
            const selected = selectedPaths.includes(file.path);
            return (
              <div className={`data-row ${selected ? "is-selected" : ""}`} key={file.path}>
                <label className="large-file-check"><input type="checkbox" checked={selected} onChange={() => togglePath(file.path)} aria-label={`Select ${file.name}`} /><span /></label>
                <div className="path-cell"><Document24Regular /><span><strong>{file.name}</strong><small>{file.path}</small></span></div>
                <strong>{formatBytes(file.sizeBytes)}</strong>
                <span>{file.lastUsedDays == null ? "Unknown" : `${file.lastUsedDays} days ago`}</span>
                <button className="large-file-ai-button" type="button" onClick={() => askAi(file)} disabled={assessmentBusy}><Sparkle24Regular />Ask AI</button>
              </div>
            );
          })}
        </div>
      </section>
      <p className="large-file-privacy-note"><ShieldCheckmark24Regular />Ask AI sends only the selected file's minimized metadata and relative location. File contents and the absolute user path stay on this PC.</p>
      {confirming && <DeleteLargeFilesDialog files={selectedFiles} busy={deleteBusy} error={deleteError} onCancel={() => { setConfirming(false); setDeleteError(""); }} onConfirm={deleteSelected} />}
      {assessmentFile && <FileAssessmentDialog file={assessmentFile} envelope={assessmentEnvelope} busy={assessmentBusy} error={assessmentError} onClose={() => { setAssessmentFile(null); setAssessmentEnvelope(null); setAssessmentError(""); }} />}
    </>
  );
}
