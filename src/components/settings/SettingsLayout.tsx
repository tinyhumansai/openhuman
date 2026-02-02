import { ReactNode, useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';

interface SettingsLayoutProps {
  children: ReactNode;
  onClose: () => void;
}

const SettingsLayout = ({ children, onClose }: SettingsLayoutProps) => {
  const modalRef = useRef<HTMLDivElement>(null);

  // Handle escape key
  useEffect(() => {
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        onClose();
      }
    };

    document.addEventListener('keydown', handleEscape);
    return () => document.removeEventListener('keydown', handleEscape);
  }, [onClose]);

  // Handle backdrop click
  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) {
      onClose();
    }
  };

  // Focus management for accessibility
  useEffect(() => {
    const previousFocus = document.activeElement as HTMLElement;

    // Focus the modal container
    if (modalRef.current) {
      modalRef.current.focus();
    }

    // Restore focus when modal closes
    return () => {
      if (previousFocus && previousFocus.focus) {
        previousFocus.focus();
      }
    };
  }, []);

  const modalContent = (
    <div
      className="fixed inset-0 z-[9999] bg-black/50 backdrop-blur-lg flex items-center justify-center p-4"
      onClick={handleBackdropClick}
      role="dialog"
      aria-modal="true"
      aria-labelledby="settings-modal-title">
      <div
        ref={modalRef}
        className="bg-stone-900 rounded-3xl shadow-large w-full max-w-[520px] max-h-[90vh] overflow-hidden animate-slide-right focus:outline-none focus:ring-0"
        style={{
          animationDuration: '300ms',
          animationTimingFunction: 'cubic-bezier(0.25, 0.46, 0.45, 0.94)',
          animationFillMode: 'both',
        }}
        tabIndex={-1}
        onClick={e => e.stopPropagation()}>
        {children}
      </div>
    </div>
  );

  return createPortal(modalContent, document.body);
};

export default SettingsLayout;
