;;; lisp-sitter.el --- Structural editing via the lisp-sitter CLI -*- lexical-binding: t; -*-

;; Author: lisp-sitter contributors
;; Version: 0.2.0
;; Package-Requires: ((emacs "27.1"))
;; Keywords: languages, tools, lisp
;; URL: https://github.com/etyurkin/lisp-sitter

;;; Commentary:

;; A thin Emacs wrapper around the `lisp-sitter' command-line tool.  It shells
;; out to the binary so that structural edits (replace/rename/format) go through
;; the same tree-sitter parse-and-validate cycle as the CLI and MCP server,
;; instead of line-based editing.
;;
;; The CLI does the structural work; this package only locates the current
;; top-level form, calls the binary, and refreshes the buffer.
;;
;; Quick start:
;;
;;   (require 'lisp-sitter)
;;   (add-hook 'emacs-lisp-mode-hook #'lisp-sitter-mode)
;;
;; For a discoverable menu of all commands, call `lisp-sitter-dispatch'
;; (requires the `transient' package, included with Emacs 28+):
;;
;;   (define-key lisp-sitter-mode-map (kbd "C-c s .") #'lisp-sitter-dispatch)
;;
;; Commands:
;;
;;   Navigate / inspect
;;   `lisp-sitter-tree'           outline of top-level forms
;;   `lisp-sitter-get'            show the text of a form
;;   `lisp-sitter-context'        structural context around point
;;   `lisp-sitter-find-errors'    list structural errors in the file
;;
;;   Edit (in-place, --write)
;;   `lisp-sitter-replace-defun'  replace the form at point from the buffer
;;   `lisp-sitter-rename'         rename a symbol (C-u: project-wide)
;;   `lisp-sitter-substitute'     replace a sub-expression inside a form
;;   `lisp-sitter-format-buffer'  re-indent the file
;;
;;   Analysis
;;   `lisp-sitter-check'          validate the file
;;   `lisp-sitter-analyze'        semantic analysis (C-u: project-wide)
;;
;;   Dispatch
;;   `lisp-sitter-dispatch'       transient menu (Emacs 28+ / transient package)

;;; Code:

(require 'subr-x)

(defgroup lisp-sitter nil
  "Structural editing through the lisp-sitter CLI."
  :group 'tools
  :prefix "lisp-sitter-")

(defcustom lisp-sitter-executable "lisp-sitter"
  "Name or path of the lisp-sitter binary."
  :type 'string
  :group 'lisp-sitter)

(defcustom lisp-sitter-check-on-save nil
  "When non-nil, run `lisp-sitter check' after saving in `lisp-sitter-mode'."
  :type 'boolean
  :group 'lisp-sitter)

(defconst lisp-sitter--extensions '("el" "lisp" "cl" "scm" "ss" "sld")
  "File extensions lisp-sitter understands.")

;;; ── process plumbing ──────────────────────────────────────────────

(defun lisp-sitter--require-file ()
  "Return the visited file name, or signal if the buffer is not a saved file."
  (or buffer-file-name
      (user-error "Buffer is not visiting a file")))

(defun lisp-sitter--run (&rest args)
  "Run the CLI with ARGS, returning (cons EXIT-CODE OUTPUT).
OUTPUT contains both stdout and stderr."
  (with-temp-buffer
    (let ((code (apply #'call-process lisp-sitter-executable nil t nil args)))
      (cons code (buffer-string)))))

(defun lisp-sitter--run-stdin (input &rest args)
  "Run the CLI with ARGS, sending INPUT on stdin.
Return (cons EXIT-CODE OUTPUT)."
  (with-temp-buffer
    (insert input)
    (let ((code (apply #'call-process-region (point-min) (point-max)
                       lisp-sitter-executable nil t nil args)))
      (cons code (buffer-string)))))

(defun lisp-sitter--check-ok (result who)
  "Signal a `user-error' when RESULT (cons CODE OUTPUT) is a failure for WHO."
  (unless (zerop (car result))
    (user-error "%s failed: %s" who (string-trim (cdr result))))
  result)

;;; ── locating the form at point ────────────────────────────────────

(defun lisp-sitter--defun-name ()
  "Return the name of the top-level form surrounding point, or nil."
  (save-excursion
    (beginning-of-defun)
    (when (looking-at "(")
      (forward-char 1)
      (skip-chars-forward " \t\n")
      ;; skip the head keyword (defun, define, …)
      (skip-chars-forward "^ \t\n()")
      (skip-chars-forward " \t\n")
      ;; a curried Scheme signature: (define (name …) …)
      (when (looking-at "(")
        (forward-char 1)
        (skip-chars-forward " \t\n"))
      (let ((start (point)))
        (skip-chars-forward "^ \t\n()")
        (when (> (point) start)
          (buffer-substring-no-properties start (point)))))))

(defun lisp-sitter--defun-text ()
  "Return the text of the top-level form surrounding point."
  (save-excursion
    (beginning-of-defun)
    (let ((start (point)))
      (end-of-defun)
      (string-trim (buffer-substring-no-properties start (point))))))

(defun lisp-sitter--read-symbol (prompt)
  "Read a symbol name with PROMPT, defaulting to the form at point."
  (let ((default (lisp-sitter--defun-name)))
    (read-string
     (if default (format "%s (default %s): " prompt default) (format "%s: " prompt))
     nil nil default)))

;;; ── output buffer ─────────────────────────────────────────────────

(defun lisp-sitter--show (name text)
  "Display TEXT in a help-style buffer called NAME."
  (let ((buf (get-buffer-create name)))
    (with-current-buffer buf
      (let ((inhibit-read-only t))
        (erase-buffer)
        (insert text)
        (goto-char (point-min)))
      (special-mode))
    (display-buffer buf)))

;;; ── commands ──────────────────────────────────────────────────────

;;;###autoload
(defun lisp-sitter-tree ()
  "Show the outline of top-level forms in the current file."
  (interactive)
  (let* ((file (lisp-sitter--require-file))
         (res (lisp-sitter--check-ok (lisp-sitter--run "tree" file) "tree")))
    (lisp-sitter--show "*lisp-sitter tree*" (cdr res))))

;;;###autoload
(defun lisp-sitter-get (symbol)
  "Show the full text of the form named SYMBOL."
  (interactive (list (lisp-sitter--read-symbol "Get form")))
  (let* ((file (lisp-sitter--require-file))
         (res (lisp-sitter--check-ok (lisp-sitter--run "get" file symbol) "get")))
    (lisp-sitter--show "*lisp-sitter form*" (cdr res))))

;;;###autoload
(defun lisp-sitter-replace-defun ()
  "Replace the top-level form at point, routing it through the CLI.
The form text is taken from the buffer and re-validated by lisp-sitter
before the file is rewritten on disk; the buffer is then reverted."
  (interactive)
  (let ((file (lisp-sitter--require-file))
        (name (or (lisp-sitter--defun-name) (user-error "No form at point")))
        (text (lisp-sitter--defun-text)))
    (when (buffer-modified-p)
      (user-error "Save the buffer first"))
    (lisp-sitter--check-ok
     (lisp-sitter--run-stdin text "replace" file name "--body-file" "-" "--write")
     "replace")
    (revert-buffer t t t)
    (message "Replaced `%s'" name)))

;;;###autoload
(defun lisp-sitter-rename (old new project)
  "Rename OLD to NEW.  With prefix arg PROJECT, rename across the directory."
  (interactive
   (list (lisp-sitter--read-symbol "Rename")
         (read-string "New name: ")
         current-prefix-arg))
  (let* ((file (lisp-sitter--require-file))
         (target (if project (file-name-directory file) file))
         (res (lisp-sitter--check-ok
               (lisp-sitter--run "rename" target old new "--write") "rename")))
    (when (and buffer-file-name (not (buffer-modified-p)))
      (revert-buffer t t t))
    (message "%s" (string-trim (cdr res)))))

;;;###autoload
(defun lisp-sitter-format-buffer ()
  "Re-indent the current file with `lisp-sitter fmt --write'."
  (interactive)
  (let ((file (lisp-sitter--require-file)))
    (when (buffer-modified-p)
      (user-error "Save the buffer first"))
    (lisp-sitter--check-ok (lisp-sitter--run "fmt" file "--write") "fmt")
    (revert-buffer t t t)
    (message "Formatted %s" (file-name-nondirectory file))))

;;;###autoload
(defun lisp-sitter-check ()
  "Validate the current file, reporting the result in the echo area."
  (interactive)
  (let* ((file (lisp-sitter--require-file))
         (res (lisp-sitter--run "check" file)))
    (if (zerop (car res))
        (message "lisp-sitter: %s" (string-trim (cdr res)))
      (lisp-sitter--show "*lisp-sitter check*" (cdr res))
      (message "lisp-sitter: check failed"))))

;;;###autoload
(defun lisp-sitter-analyze (project)
  "Run semantic analysis on the current file.
With prefix arg PROJECT, analyze the whole directory."
  (interactive "P")
  (let* ((file (lisp-sitter--require-file))
         (target (if project (file-name-directory file) file))
         (res (lisp-sitter--run "analyze" target)))
    (lisp-sitter--show "*lisp-sitter analyze*" (cdr res))))

;;;###autoload
(defun lisp-sitter-context ()
  "Show the structural context (outline, bounds, full text) of the current file."
  (interactive)
  (let* ((file (lisp-sitter--require-file))
         (res (lisp-sitter--check-ok (lisp-sitter--run "context" file) "context")))
    (lisp-sitter--show "*lisp-sitter context*" (cdr res))))

;;;###autoload
(defun lisp-sitter-find-errors ()
  "List structural errors (missing tokens, unbalanced parens) in the current file."
  (interactive)
  (let* ((file (lisp-sitter--require-file))
         (res (lisp-sitter--run "find-errors" file)))
    (if (string-blank-p (string-trim (cdr res)))
        (message "lisp-sitter: no structural errors found")
      (lisp-sitter--show "*lisp-sitter errors*" (cdr res)))))

;;;###autoload
(defun lisp-sitter-substitute (symbol old new)
  "Replace sub-expression OLD with NEW inside the form named SYMBOL.
Applies the change to the file on disk and reverts the buffer."
  (interactive
   (list (lisp-sitter--read-symbol "In form")
         (read-string "Replace pattern: ")
         (read-string "With: ")))
  (let ((file (lisp-sitter--require-file)))
    (when (buffer-modified-p)
      (user-error "Save the buffer first"))
    (lisp-sitter--check-ok
     (lisp-sitter--run "substitute" file symbol old new "--write") "substitute")
    (revert-buffer t t t)
    (message "Substituted in `%s'" symbol)))

;;; ── minor mode ────────────────────────────────────────────────────

(defun lisp-sitter--maybe-check-on-save ()
  "Run `lisp-sitter-check' after save when `lisp-sitter-check-on-save' is set."
  (when (and lisp-sitter-check-on-save
             buffer-file-name
             (member (file-name-extension buffer-file-name) lisp-sitter--extensions))
    (lisp-sitter-check)))

(defvar lisp-sitter-mode-map
  (let ((map (make-sparse-keymap)))
    ;; Navigate / inspect
    (define-key map (kbd "C-c s t") #'lisp-sitter-tree)
    (define-key map (kbd "C-c s g") #'lisp-sitter-get)
    (define-key map (kbd "C-c s x") #'lisp-sitter-context)
    (define-key map (kbd "C-c s e") #'lisp-sitter-find-errors)
    ;; Edit
    (define-key map (kbd "C-c s r") #'lisp-sitter-replace-defun)
    (define-key map (kbd "C-c s R") #'lisp-sitter-rename)
    (define-key map (kbd "C-c s s") #'lisp-sitter-substitute)
    (define-key map (kbd "C-c s f") #'lisp-sitter-format-buffer)
    ;; Analysis
    (define-key map (kbd "C-c s c") #'lisp-sitter-check)
    (define-key map (kbd "C-c s a") #'lisp-sitter-analyze)
    ;; Dispatch
    (define-key map (kbd "C-c s .") #'lisp-sitter-dispatch)
    map)
  "Keymap for `lisp-sitter-mode'.")

;;;###autoload
(define-minor-mode lisp-sitter-mode
  "Minor mode for structural Lisp editing via the lisp-sitter CLI."
  :lighter " ls"
  :keymap lisp-sitter-mode-map
  (if lisp-sitter-mode
      (add-hook 'after-save-hook #'lisp-sitter--maybe-check-on-save nil t)
    (remove-hook 'after-save-hook #'lisp-sitter--maybe-check-on-save t)))

;;; ── transient dispatch menu ───────────────────────────────────────

(defun lisp-sitter--transient-available-p ()
  "Return non-nil when the `transient' package is loadable."
  (require 'transient nil t))

;; Define the prefix lazily so that the file loads cleanly on Emacs 27 (where
;; transient ships as a third-party package and may not be installed).
(defun lisp-sitter--define-dispatch ()
  "Define `lisp-sitter-dispatch' using transient, then invoke it."
  (transient-define-prefix lisp-sitter-dispatch ()
    "Structural Lisp editing via lisp-sitter."
    ["Navigate / inspect"
     ("t" "Outline (tree)"          lisp-sitter-tree)
     ("g" "Get form text"           lisp-sitter-get)
     ("x" "Structural context"      lisp-sitter-context)
     ("e" "Find structural errors"  lisp-sitter-find-errors)]
    ["Edit (writes file)"
     ("r" "Replace form at point"   lisp-sitter-replace-defun)
     ("R" "Rename symbol"           lisp-sitter-rename)
     ("s" "Substitute sub-expr"     lisp-sitter-substitute)
     ("f" "Format buffer"           lisp-sitter-format-buffer)]
    ["Analysis"
     ("c" "Check (validate)"        lisp-sitter-check)
     ("a" "Analyze (semantic)"      lisp-sitter-analyze)])
  ;; Replace this indirection with the real command for subsequent calls.
  (fset 'lisp-sitter-dispatch (symbol-function 'lisp-sitter-dispatch))
  (lisp-sitter-dispatch))

;;;###autoload
(defun lisp-sitter-dispatch ()
  "Show the lisp-sitter command menu (requires the transient package)."
  (interactive)
  (if (lisp-sitter--transient-available-p)
      (lisp-sitter--define-dispatch)
    (user-error
     "lisp-sitter-dispatch requires the `transient' package (included with Emacs 28+)")))

(provide 'lisp-sitter)
;;; lisp-sitter.el ends here
