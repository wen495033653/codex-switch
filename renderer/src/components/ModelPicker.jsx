// The test model dropdown of the API precheck and of the subscription downgrade test.
export default function ModelPicker({ disabled, onClose, onSelect, open, options, setOpen, value }) {
  return (
    <div
      className="api-check-model-picker"
      onBlur={event => {
        if (!event.currentTarget.contains(event.relatedTarget)) onClose();
      }}
    >
      <button
        type="button"
        className="api-check-model-trigger"
        disabled={disabled}
        onClick={() => setOpen(current => !current)}
        aria-haspopup="listbox"
        aria-expanded={open}
      >
        <span title={value}>{value}</span>
        <svg aria-hidden="true" viewBox="0 0 20 20" fill="currentColor">
          <path fillRule="evenodd" d="M5.23 7.21a.75.75 0 0 1 1.06.02L10 11.168l3.71-3.938a.75.75 0 1 1 1.08 1.04l-4.25 4.5a.75.75 0 0 1-1.08 0l-4.25-4.5a.75.75 0 0 1 .02-1.06Z" clipRule="evenodd" />
        </svg>
      </button>
      {open && !disabled && (
        <div className="api-check-model-menu" role="listbox">
          {options.map(model => {
            const selected = model === value;
            return (
              <button
                key={model}
                type="button"
                className={`api-check-model-option ${selected ? 'selected' : ''}`}
                role="option"
                aria-selected={selected}
                onClick={() => onSelect(model)}
              >
                {model}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
