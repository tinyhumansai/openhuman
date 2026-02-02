import { useEffect, useState } from 'react';

interface TypewriterGreetingProps {
  greetings: string[];
  typingSpeed?: number;
  deletingSpeed?: number;
  pauseTime?: number;
  className?: string;
}

const TypewriterGreeting = ({
  greetings,
  typingSpeed = 100,
  deletingSpeed = 50,
  pauseTime = 2000,
  className = '',
}: TypewriterGreetingProps) => {
  const [currentGreetingIndex, setCurrentGreetingIndex] = useState(0);
  const [displayedText, setDisplayedText] = useState('');
  const [isDeleting, setIsDeleting] = useState(false);
  const [isPaused, setIsPaused] = useState(false);

  useEffect(() => {
    if (greetings.length === 0) return;

    const currentGreeting = greetings[currentGreetingIndex];
    const speed = isDeleting ? deletingSpeed : typingSpeed;

    if (isPaused) {
      const pauseTimer = setTimeout(() => {
        setIsPaused(false);
        setIsDeleting(true);
      }, pauseTime);
      return () => clearTimeout(pauseTimer);
    }

    const timer = setTimeout(() => {
      if (!isDeleting) {
        // Typing
        if (displayedText.length < currentGreeting.length) {
          setDisplayedText(currentGreeting.slice(0, displayedText.length + 1));
        } else {
          // Finished typing, pause before deleting
          setIsPaused(true);
        }
      } else {
        // Deleting
        if (displayedText.length > 0) {
          setDisplayedText(displayedText.slice(0, -1));
        } else {
          // Finished deleting, move to next greeting
          setIsDeleting(false);
          setCurrentGreetingIndex(prev => (prev + 1) % greetings.length);
        }
      }
    }, speed);

    return () => clearTimeout(timer);
  }, [
    displayedText,
    isDeleting,
    isPaused,
    currentGreetingIndex,
    greetings,
    typingSpeed,
    deletingSpeed,
    pauseTime,
  ]);

  return (
    <h1 className={`text-2xl font-bold mb-4 ${className}`}>
      {displayedText}
      <span className="animate-pulse">|</span>
    </h1>
  );
};

export default TypewriterGreeting;
