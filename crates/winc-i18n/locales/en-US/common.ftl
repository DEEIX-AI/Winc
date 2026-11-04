## Text shared across areas: common buttons and options.

common-cancel = Cancel
common-open = Open
common-save = Save
common-save-as-label = Save As:
common-list-separator = {", "}
common-reveal-in-finder = Reveal in Finder
common-reveal-in-folder = { $platform ->
    [windows] Reveal in File Explorer
   *[other] Show in Folder
}
common-close = Close
common-close-dialog-first = A dialog is already open in this window; close it first

## Move to Trash

common-trash = { $platform ->
    [windows] Recycle Bin
   *[other] Trash
}
common-trash-confirm-folder = Move the folder “{ $name }” and its contents to the { common-trash }?
common-trash-confirm-item = Move “{ $name }” to the { common-trash }?
common-trash-confirm-items = { $count ->
        [one] Move this item to the { common-trash }?
       *[other] Move these { $count } items to the { common-trash }?
    }
common-trash-confirm-detail = You can restore items from the { common-trash }.
common-trash-move = Move to { common-trash }
common-trash-moved-item = Moved to { common-trash }: { $name }
common-trash-moved-items = { $count ->
        [one] Moved { $count } item to the { common-trash }
       *[other] Moved { $count } items to the { common-trash }
    }
common-trash-delete-failed = Couldn’t delete “{ $name }”: { $error }
common-trash-invalid-path = Invalid path
common-trash-no-file-manager = NSFileManager is unavailable
common-trash-failed = Couldn’t move to the { common-trash }
common-trash-failed-code = Couldn’t move to the { common-trash } (error { $code })
common-trash-cancelled = Cancelled; nothing was moved to the { common-trash }
common-trash-unsupported = Moving to the Recycle Bin isn’t supported on this platform

## Opening links (terminal, Markdown preview)
common-link-scheme-not-allowed = Only http, https, and mailto links are supported

## New Worktree and New Branch dialogs

common-dialog-new-worktree = New Worktree
common-dialog-new-branch = New Branch
common-dialog-branch = Branch
common-dialog-name = Name
common-dialog-base = Based On
common-dialog-location = Location
common-dialog-worktree-placeholder = New or existing branch name
common-dialog-branch-placeholder = New branch name
common-dialog-branch-name = branch-name
common-dialog-copy-ignored = Copy Ignored Files
common-dialog-open-in-new-window = Open in New Window
common-dialog-checkout-after-create = Check Out After Creating
common-dialog-creating = Creating…
