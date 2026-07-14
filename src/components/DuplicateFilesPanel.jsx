import { useEffect, useMemo, useState } from "react";
import {
  Delete24Regular,
  Dismiss24Regular,
  DocumentCopy24Regular,
  ShieldCheckmark24Regular,
  Sparkle24Regular,
  Warning24Regular,
} from "@fluentui/react-icons";
import { formatBytes } from "../lib/format.js";

const recommendationLabels = {
  delete: "Reasonable to delete",
  keep: "Keep this copy",
  review: "Review before deleting",
};

function DeleteDuplicateDialog({ selectedFiles, selectedBytes, deleting, error, onCancel, onConfirm }) {
  return (
    <div className="modal-backdrop duplicate-delete-backdrop" role="presentation" onMouseDown={(event) => {
      if (!deleting && event.target === event.currentTarget) onCancel();
    }}>
      <section className="duplicate-delete-dialog" role="dialog" aria-modal="true" aria-labelledby="duplicate-delete-title">
        <header>
          <span><Delete24Regular /></span>
          <div>
            <small>Permanent deletion</small>
            <h2 id="duplicate-delete-title">Delete {selectedFiles.length} selected {selectedFiles.length === 1 ? "copy" : "copies"}?</h2>
            <p>This removes {formatBytes(selectedBytes)} from disk. The action cannot be undone.</p>
          </div>
          <button className="icon-button" type="button" aria-label="Close deletion confirmation" disabled={deleting} onClick={onCancel}><Dismiss24Regular /></button>
        </header>
        <div className="duplicate-delete-files">
          {selectedFiles.slice(0, 5).map((file) => <span key={file.path}>{file.path}</span>)}
          {selectedFiles.length > 5 && <small>And {selectedFiles.length - 5} more selected files</small>}
        </div>
        <div className="duplicate-delete-safety"><ShieldCheckmark24Regular /><span><strong>One copy stays in every group.</strong> Luna checks the retained copy, file size, and content hash again before deleting anything.</span></div>
        {error && <p className="duplicate-action-error"><Warning24Regular />{error}</p>}
        <footer>
          <button className="secondary-button" type="button" disabled={deleting} onClick={onCancel}>Keep files</button>
          <button className="danger-button" type="button" disabled={deleting} onClick={onConfirm}><Delete24Regular />{deleting ? "Verifying and deleting…" : "Delete selected files"}</button>
        </footer>
      </section>
    </div>
  );
}

function AiDuplicateReview({ envelope }) {
  const { review } = envelope;
  return (
    <section className={`duplicate-ai-review recommendation-${review.recommendation}`} aria-live="polite">
      <header>
        <span><Sparkle24Regular /></span>
        <div><small>AI opinion · {review.confidence} confidence</small><h3>{review.headline}</h3></div>
        <b>{recommendationLabels[review.recommendation] || "Review carefully"}</b>
      </header>
      <p>{review.summary}</p>
      <div className="duplicate-ai-columns">
        <div><strong>Why</strong><ul>{review.reasons.map((reason) => <li key={reason}>{reason}</li>)}</ul></div>
        <div><strong>What to do</strong><ul>{review.suggestions.map((suggestion) => <li key={suggestion}>{suggestion}</li>)}</ul></div>
      </div>
      <footer>Metadata-only opinion · {review.riskLevel} risk · {envelope.model}</footer>
    </section>
  );
}

export function DuplicateFilesPanel({ result, onChooseFolder, onDeleteFiles, onAskAi }) {
  const [selectedPaths, setSelectedPaths] = useState(() => new Set());
  const [selectionError, setSelectionError] = useState("");
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState("");
  const [aiPath, setAiPath] = useState("");
  const [aiBusyPath, setAiBusyPath] = useState("");
  const [aiReview, setAiReview] = useState(null);
  const [aiError, setAiError] = useState("");

  const allFiles = useMemo(() => result.duplicateGroups.flatMap((group) => group.files.map((file) => ({ ...file, contentHash: group.contentHash, sizeBytes: group.sizeBytes }))), [result]);
  const selectedFiles = allFiles.filter((file) => selectedPaths.has(file.path));
  const selectedBytes = selectedFiles.reduce((total, file) => total + file.sizeBytes, 0);
  const reclaimable = result.duplicateGroups.reduce((sum, group) => sum + group.reclaimableBytes, 0);

  useEffect(() => {
    const currentPaths = new Set(allFiles.map((file) => file.path));
    setSelectedPaths((current) => new Set([...current].filter((path) => currentPaths.has(path))));
    if (aiPath && !currentPaths.has(aiPath)) {
      setAiPath("");
      setAiReview(null);
    }
  }, [allFiles, aiPath]);

  function toggleFile(group, path) {
    setSelectionError("");
    setSelectedPaths((current) => {
      const next = new Set(current);
      if (next.has(path)) {
        next.delete(path);
        return next;
      }
      const selectedInGroup = group.files.filter((file) => next.has(file.path)).length;
      if (selectedInGroup >= group.files.length - 1) {
        setSelectionError("Keep at least one file from every duplicate group. Deselect another copy first if you want to keep this one instead.");
        return current;
      }
      next.add(path);
      return next;
    });
  }

  async function askAi(group, file) {
    if (!onAskAi || aiBusyPath) return;
    setAiPath(file.path);
    setAiReview(null);
    setAiError("");
    setAiBusyPath(file.path);
    try {
      const envelope = await onAskAi({ contentHash: group.contentHash, path: file.path });
      if (envelope) setAiReview(envelope);
    } catch (error) {
      setAiError(String(error));
    } finally {
      setAiBusyPath("");
    }
  }

  async function deleteSelected() {
    if (!onDeleteFiles || !selectedFiles.length || deleting) return;
    setDeleting(true);
    setDeleteError("");
    try {
      const groups = result.duplicateGroups
        .map((group) => ({
          contentHash: group.contentHash,
          paths: group.files.filter((file) => selectedPaths.has(file.path)).map((file) => file.path),
        }))
        .filter((group) => group.paths.length > 0);
      const outcome = await onDeleteFiles({ groups });
      const deleted = new Set(outcome.deletedFiles.map((file) => file.path));
      setSelectedPaths((current) => new Set([...current].filter((path) => !deleted.has(path))));
      if (outcome.failures.length > 0) {
        setDeleteError(`${outcome.failures.length} selected ${outcome.failures.length === 1 ? "file was" : "files were"} left untouched. ${outcome.failures[0].reason}`);
      }
      if (outcome.deletedFiles.length > 0) setConfirmingDelete(false);
    } catch (error) {
      setDeleteError(String(error));
    } finally {
      setDeleting(false);
    }
  }

  return (
    <div className="feature-view">
      <header className="feature-header">
        <div><span>Duplicates</span><h1>{formatBytes(reclaimable)} in exact copies</h1><p>Select only the copies you want to remove. Luna always leaves at least one file from each verified group.</p></div>
        <button className="secondary-button" type="button" onClick={onChooseFolder}>Scan another folder</button>
      </header>

      {result.duplicateGroups.length === 0 ? (
        <section className="empty-state-surface"><DocumentCopy24Regular /><h2>No exact duplicates in this scan</h2><p>Try a broader folder or drive if you want to search elsewhere.</p></section>
      ) : (
        <>
          <section className="duplicate-selection-bar" aria-live="polite">
            <div><strong>{selectedFiles.length} {selectedFiles.length === 1 ? "file" : "files"} selected</strong><span>{selectedFiles.length ? `${formatBytes(selectedBytes)} will be removed after verification.` : "Nothing is selected automatically."}</span></div>
            <button className="danger-button" type="button" disabled={!selectedFiles.length} onClick={() => {
              setDeleteError("");
              setConfirmingDelete(true);
            }}><Delete24Regular /> Delete selected</button>
          </section>
          {selectionError && <p className="duplicate-action-error"><Warning24Regular />{selectionError}</p>}
          <section className="feature-surface duplicate-list">
            {result.duplicateGroups.map((group, index) => {
              const groupSelected = group.files.filter((file) => selectedPaths.has(file.path)).length;
              return (
                <details key={group.contentHash} open={index === 0}>
                  <summary><span><DocumentCopy24Regular /></span><div><strong>{group.files[0]?.name || "Duplicate group"}</strong><small>{group.files.length} identical files · {groupSelected ? `${groupSelected} selected · ` : ""}hash {group.contentHash.slice(0, 12)}…</small></div><b>{formatBytes(group.reclaimableBytes)} reclaimable</b></summary>
                  <div>
                    {group.files.map((file) => {
                      const selected = selectedPaths.has(file.path);
                      const reviewing = aiPath === file.path;
                      return (
                        <div className={`duplicate-file-entry ${selected ? "is-selected" : ""}`} key={file.path}>
                          <label className="duplicate-file-select">
                            <input type="checkbox" checked={selected} onChange={() => toggleFile(group, file.path)} />
                            <span><strong>{file.name}</strong><small title={file.path}>{file.path}</small></span>
                          </label>
                          <span className="duplicate-file-age">{file.lastUsedDays == null ? "Activity unknown" : `${file.lastUsedDays} days ago`}</span>
                          <button className="duplicate-ask-button" type="button" disabled={Boolean(aiBusyPath)} onClick={() => askAi(group, file)}><Sparkle24Regular />{aiBusyPath === file.path ? "Asking AI…" : "Ask AI"}</button>
                          {reviewing && aiError && <p className="duplicate-ai-error"><Warning24Regular />{aiError}</p>}
                          {reviewing && aiReview && <AiDuplicateReview envelope={aiReview} />}
                        </div>
                      );
                    })}
                  </div>
                </details>
              );
            })}
          </section>
        </>
      )}

      {confirmingDelete && <DeleteDuplicateDialog selectedFiles={selectedFiles} selectedBytes={selectedBytes} deleting={deleting} error={deleteError} onCancel={() => setConfirmingDelete(false)} onConfirm={deleteSelected} />}
    </div>
  );
}
