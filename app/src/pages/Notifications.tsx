import NotificationCenter from '../components/notifications/NotificationCenter';

/**
 * Full-page notification center route (`/notifications`).
 */
const Notifications = () => {
  return (
    <div className="h-full flex flex-col">
      <NotificationCenter />
    </div>
  );
};

export default Notifications;
