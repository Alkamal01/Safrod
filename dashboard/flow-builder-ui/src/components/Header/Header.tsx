import styles from './Header.module.scss';

export function Header() {
  return (
    <div className={styles.section}>
      <div className={styles.heading}>
        <div className={styles.logo} aria-label="Safrod">◈ Safrod</div>
        <div className={styles.subtitle}>Build your flow</div>
      </div>
      <div className={styles.description}>
        <p className={styles.body}>
          Explore Safrod’s Bitcoin and Lightning APIs. Select a source and destination to see the calls for your local test flow.
        </p>
      </div>
    </div>
  );
}
