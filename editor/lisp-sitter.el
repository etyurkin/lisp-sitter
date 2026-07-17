;;; lisp-sitter.el --- Structural editing via the lisp-sitter CLI -*- lexical-binding: t; -*-

;; Author: lisp-sitter contributors
;; Version: 1.1.2
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
;;   `lisp-sitter-callers'        callers of a symbol (C-u: project)
;;   `lisp-sitter-callees'        callees of a symbol (C-u: project)
;;   `lisp-sitter-explore'        source + callers + callees
;;   `lisp-sitter-impact'         transitive callers
;;
;;   Edit (in-place, --write)
;;   `lisp-sitter-replace-defun'  replace the form at point from the buffer
;;   `lisp-sitter-rename'         rename a symbol (C-u: project-wide)
;;   `lisp-sitter-substitute'     replace a sub-expression inside a form
;;   `lisp-sitter-wrap'           wrap a form body
;;   `lisp-sitter-extract'        extract a sub-expression
;;   `lisp-sitter-move'           move a form after an anchor
;;   `lisp-sitter-remove'         remove a form
;;   `lisp-sitter-insert'         insert a form after an anchor
;;   `lisp-sitter-flatten'        inline a function (C-u: project)
;;   `lisp-sitter-splice'         paredit splice
;;   `lisp-sitter-raise'          paredit raise
;;   `lisp-sitter-slurp'          paredit slurp
;;   `lisp-sitter-barf'           paredit barf
;;   `lisp-sitter-convert-let'    convert let/let*
;;   `lisp-sitter-instrument'     instrument with tracing
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

(defun lisp-sitter--project-or-file (project)
  "Return the current file, or its directory when PROJECT is non-nil."
  (let ((file (lisp-sitter--require-file)))
    (if project (file-name-directory file) file)))

(defun lisp-sitter--write-and-revert (who &rest args)
  "Run CLI ARGS with --write, requiring a clean buffer, then revert."
  (when (buffer-modified-p)
    (user-error "Save the buffer first"))
  (lisp-sitter--check-ok (apply #'lisp-sitter--run args) who)
  (revert-buffer t t t))

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
The form text is taken from the current buffer (including unsaved edits to
that form) and re-validated by lisp-sitter before the file is rewritten on
disk.  When the buffer has no other unsaved changes it is reverted to reflect
any normalization; otherwise it is left untouched so unrelated edits survive."
  (interactive)
  (let ((file (lisp-sitter--require-file))
        (name (or (lisp-sitter--defun-name) (user-error "No form at point")))
        (text (lisp-sitter--defun-text)))
    (lisp-sitter--check-ok
     (lisp-sitter--run-stdin text "replace" file name "--body-file" "-" "--write")
     "replace")
    (if (buffer-modified-p)
        (message "Replaced `%s' on disk; buffer has other unsaved edits" name)
      (revert-buffer t t t)
      (message "Replaced `%s'" name))))

;;;###autoload
(defun lisp-sitter-rename (old new project)
  "Rename OLD to NEW.  With prefix arg PROJECT, rename across the directory."
  (interactive
   (list (lisp-sitter--read-symbol "Rename")
         (read-string "New name: ")
         current-prefix-arg))
  (when (buffer-modified-p)
    (user-error "Save the buffer first"))
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
         (res (lisp-sitter--run "find-errors" file))
         (out (string-trim (cdr res))))
    (if (or (string-blank-p out) (string-prefix-p "No errors" out))
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
  (lisp-sitter--write-and-revert
   "substitute" "substitute" (lisp-sitter--require-file) symbol
   "--pattern" old "--replacement" new "--write")
  (message "Substituted in `%s'" symbol))


;;;###autoload
(defun lisp-sitter-wrap (symbol wrapper)
  "Wrap the body of SYMBOL in WRAPPER (`progn', `begin', `let', or `if')."
  (interactive
   (list (lisp-sitter--read-symbol "Wrap form")
         (completing-read "Wrapper: " '("progn" "begin" "let" "if") nil t)))
  (let* ((file (lisp-sitter--require-file))
         (args (list "wrap" file symbol "--in" wrapper "--write")))
    (when (equal wrapper "let")
      (setq args (append args (list "--bindings" (read-string "Bindings: " "()")))))
    (when (equal wrapper "if")
      (setq args (append args (list "--condition" (read-string "Condition: " "t")))))
    (apply #'lisp-sitter--write-and-revert "wrap" args)
    (message "Wrapped `%s' in %s" symbol wrapper)))

;;;###autoload
(defun lisp-sitter-extract (symbol pattern name)
  "Extract PATTERN from SYMBOL into a new function named NAME."
  (interactive
   (list (lisp-sitter--read-symbol "From form")
         (read-string "Pattern: ")
         (read-string "New name: ")))
  (lisp-sitter--write-and-revert
   "extract" "extract" (lisp-sitter--require-file) symbol
   "--pattern" pattern "--name" name "--write")
  (message "Extracted `%s'" name))

;;;###autoload
(defun lisp-sitter-move (symbol after)
  "Move SYMBOL after AFTER (`__start__', `__end__', or another symbol)."
  (interactive
   (list (lisp-sitter--read-symbol "Move form")
         (read-string "After (symbol/__start__/__end__): " "__end__")))
  (lisp-sitter--write-and-revert
   "move" "move" (lisp-sitter--require-file) symbol "--after" after "--write")
  (message "Moved `%s'" symbol))

;;;###autoload
(defun lisp-sitter-remove (symbol)
  "Remove the top-level form named SYMBOL."
  (interactive (list (lisp-sitter--read-symbol "Remove form")))
  (lisp-sitter--write-and-revert
   "remove" "remove" (lisp-sitter--require-file) symbol "--write")
  (message "Removed `%s'" symbol))

;;;###autoload
(defun lisp-sitter-insert (after node)
  "Insert NODE after AFTER (`__start__', `__end__', or a symbol)."
  (interactive
   (list (read-string "After (symbol/__start__/__end__): " "__end__")
         (read-string "Form: ")))
  (lisp-sitter--write-and-revert
   "insert" "insert" (lisp-sitter--require-file) after "--node" node "--write")
  (message "Inserted after `%s'" after))

;;;###autoload
(defun lisp-sitter-flatten (symbol project)
  "Inline calls to SYMBOL and remove its definition.
With prefix arg PROJECT, flatten across the directory."
  (interactive
   (list (lisp-sitter--read-symbol "Flatten")
         current-prefix-arg))
  (let ((target (lisp-sitter--project-or-file project)))
    (lisp-sitter--write-and-revert "flatten" "flatten" target symbol "--write")
    (message "Flattened `%s'" symbol)))

;;;###autoload
(defun lisp-sitter-splice (symbol pattern)
  "Paredit splice: dissolve PATTERN inside SYMBOL."
  (interactive
   (list (lisp-sitter--read-symbol "In form")
         (read-string "Pattern: ")))
  (lisp-sitter--write-and-revert
   "splice" "splice" (lisp-sitter--require-file) symbol
   "--pattern" pattern "--write")
  (message "Spliced in `%s'" symbol))

;;;###autoload
(defun lisp-sitter-raise (symbol pattern)
  "Paredit raise: promote PATTERN inside SYMBOL."
  (interactive
   (list (lisp-sitter--read-symbol "In form")
         (read-string "Pattern: ")))
  (lisp-sitter--write-and-revert
   "raise" "raise" (lisp-sitter--require-file) symbol
   "--pattern" pattern "--write")
  (message "Raised in `%s'" symbol))

;;;###autoload
(defun lisp-sitter-slurp (symbol pattern dir)
  "Paredit slurp PATTERN inside SYMBOL in direction DIR."
  (interactive
   (list (lisp-sitter--read-symbol "In form")
         (read-string "Pattern: ")
         (completing-read "Direction: " '("forward" "backward") nil t nil nil "forward")))
  (lisp-sitter--write-and-revert
   "slurp" "slurp" (lisp-sitter--require-file) symbol
   "--pattern" pattern "--dir" dir "--write")
  (message "Slurped in `%s'" symbol))

;;;###autoload
(defun lisp-sitter-barf (symbol pattern dir)
  "Paredit barf PATTERN inside SYMBOL in direction DIR."
  (interactive
   (list (lisp-sitter--read-symbol "In form")
         (read-string "Pattern: ")
         (completing-read "Direction: " '("forward" "backward") nil t nil nil "forward")))
  (lisp-sitter--write-and-revert
   "barf" "barf" (lisp-sitter--require-file) symbol
   "--pattern" pattern "--dir" dir "--write")
  (message "Barfed in `%s'" symbol))

;;;###autoload
(defun lisp-sitter-convert-let (symbol to)
  "Convert the first let/let* in SYMBOL to TO (`let' or `let*')."
  (interactive
   (list (lisp-sitter--read-symbol "In form")
         (completing-read "Convert to: " '("let" "let*") nil t)))
  (lisp-sitter--write-and-revert
   "convert-let" "convert-let" (lisp-sitter--require-file) symbol
   "--to" to "--write")
  (message "Converted let in `%s'" symbol))

;;;###autoload
(defun lisp-sitter-instrument (symbol)
  "Instrument SYMBOL's body with a tracing form (`--with')."
  (interactive (list (lisp-sitter--read-symbol "Instrument")))
  (let ((trace (read-string "Trace form: " "(message \"trace\")")))
    (lisp-sitter--write-and-revert
     "instrument" "instrument" (lisp-sitter--require-file) symbol
     "--with" trace "--write")
    (message "Instrumented `%s'" symbol)))

;;;###autoload
(defun lisp-sitter-callers (symbol project)
  "Show callers of SYMBOL.  With prefix arg PROJECT, scan the directory."
  (interactive
   (list (lisp-sitter--read-symbol "Callers of")
         current-prefix-arg))
  (let* ((target (lisp-sitter--project-or-file project))
         (res (lisp-sitter--check-ok
               (lisp-sitter--run "callers" target symbol) "callers")))
    (lisp-sitter--show "*lisp-sitter callers*" (cdr res))))

;;;###autoload
(defun lisp-sitter-callees (symbol project)
  "Show callees of SYMBOL.  With prefix arg PROJECT, scan the directory."
  (interactive
   (list (lisp-sitter--read-symbol "Callees of")
         current-prefix-arg))
  (let* ((target (lisp-sitter--project-or-file project))
         (res (lisp-sitter--check-ok
               (lisp-sitter--run "callees" target symbol) "callees")))
    (lisp-sitter--show "*lisp-sitter callees*" (cdr res))))

;;;###autoload
(defun lisp-sitter-explore (symbol project)
  "Explore SYMBOL (source, callers, callees).  C-u for project-wide."
  (interactive
   (list (lisp-sitter--read-symbol "Explore")
         current-prefix-arg))
  (let* ((target (lisp-sitter--project-or-file project))
         (res (lisp-sitter--check-ok
               (lisp-sitter--run "explore" target symbol) "explore")))
    (lisp-sitter--show "*lisp-sitter explore*" (cdr res))))

;;;###autoload
(defun lisp-sitter-impact (symbol project)
  "Show transitive callers (blast radius) of SYMBOL.  C-u for project-wide."
  (interactive
   (list (lisp-sitter--read-symbol "Impact of")
         current-prefix-arg))
  (let* ((target (lisp-sitter--project-or-file project))
         (res (lisp-sitter--check-ok
               (lisp-sitter--run "impact" target symbol) "impact")))
    (lisp-sitter--show "*lisp-sitter impact*" (cdr res))))

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
    (define-key map (kbd "C-c s C") #'lisp-sitter-callers)
    (define-key map (kbd "C-c s E") #'lisp-sitter-explore)
    (define-key map (kbd "C-c s I") #'lisp-sitter-impact)
    ;; Edit
    (define-key map (kbd "C-c s r") #'lisp-sitter-replace-defun)
    (define-key map (kbd "C-c s R") #'lisp-sitter-rename)
    (define-key map (kbd "C-c s s") #'lisp-sitter-substitute)
    (define-key map (kbd "C-c s w") #'lisp-sitter-wrap)
    (define-key map (kbd "C-c s X") #'lisp-sitter-extract)
    (define-key map (kbd "C-c s m") #'lisp-sitter-move)
    (define-key map (kbd "C-c s d") #'lisp-sitter-remove)
    (define-key map (kbd "C-c s i") #'lisp-sitter-insert)
    (define-key map (kbd "C-c s F") #'lisp-sitter-flatten)
    (define-key map (kbd "C-c s S") #'lisp-sitter-splice)
    (define-key map (kbd "C-c s ^") #'lisp-sitter-raise)
    (define-key map (kbd "C-c s >") #'lisp-sitter-slurp)
    (define-key map (kbd "C-c s <") #'lisp-sitter-barf)
    (define-key map (kbd "C-c s L") #'lisp-sitter-convert-let)
    (define-key map (kbd "C-c s T") #'lisp-sitter-instrument)
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
     ("e" "Find structural errors"  lisp-sitter-find-errors)
     ("C" "Callers"                 lisp-sitter-callers)
     ("E" "Explore"                 lisp-sitter-explore)
     ("I" "Impact"                  lisp-sitter-impact)]
    ["Edit (writes file)"
     ("r" "Replace form at point"   lisp-sitter-replace-defun)
     ("R" "Rename symbol"           lisp-sitter-rename)
     ("s" "Substitute sub-expr"     lisp-sitter-substitute)
     ("w" "Wrap body"               lisp-sitter-wrap)
     ("X" "Extract"                 lisp-sitter-extract)
     ("m" "Move form"               lisp-sitter-move)
     ("d" "Remove form"             lisp-sitter-remove)
     ("i" "Insert form"             lisp-sitter-insert)
     ("F" "Flatten"                 lisp-sitter-flatten)
     ("S" "Splice"                  lisp-sitter-splice)
     ("^" "Raise"                   lisp-sitter-raise)
     (">" "Slurp"                   lisp-sitter-slurp)
     ("<" "Barf"                    lisp-sitter-barf)
     ("L" "Convert let"             lisp-sitter-convert-let)
     ("T" "Instrument"              lisp-sitter-instrument)
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
